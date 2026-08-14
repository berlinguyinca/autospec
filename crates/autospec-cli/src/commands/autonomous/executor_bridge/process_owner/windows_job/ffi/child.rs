use super::super::super::PreparedLaunchSpec;
use super::super::command_line::{command_line, environment_block, wide_nul};
use super::*;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;

pub(in crate::commands::autonomous::executor_bridge::process_owner) struct WindowsJobChild {
    job: OwnedHandle,
    process: OwnedHandle,
    thread: Option<OwnedHandle>,
    pid: u32,
    creation_filetime: u64,
    terminal: Option<ExitStatus>,
    job_terminated: bool,
}

impl WindowsJobChild {
    pub(in crate::commands::autonomous::executor_bridge::process_owner) fn spawn(
        spec: PreparedLaunchSpec,
    ) -> Result<Self, String> {
        let job = OwnedHandle::new(
            // SAFETY: null attributes and name request an unnamed Job Object.
            unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) },
            "create autonomous Job Object",
        )?;
        let limits = JobObjectExtendedLimitInformation {
            basic_limit_information: JobObjectBasicLimitInformation {
                per_process_user_time_limit: 0,
                per_job_user_time_limit: 0,
                limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                minimum_working_set_size: 0,
                maximum_working_set_size: 0,
                active_process_limit: 0,
                affinity: 0,
                priority_class: 0,
                scheduling_class: 0,
            },
            io_info: IoCounters {
                read_operation_count: 0,
                write_operation_count: 0,
                other_operation_count: 0,
                read_transfer_count: 0,
                write_transfer_count: 0,
                other_transfer_count: 0,
            },
            process_memory_limit: 0,
            job_memory_limit: 0,
            peak_process_memory_used: 0,
            peak_job_memory_used: 0,
        };
        // SAFETY: limits has the exact ABI layout and remains live for the call.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                (&limits as *const JobObjectExtendedLimitInformation).cast(),
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        } == 0
        {
            return Err(last_error("configure autonomous Job Object"));
        }

        let stdin = duplicate_inheritable(spec.stdin.as_ref().map_or_else(
            || unsafe { GetStdHandle(STD_INPUT_HANDLE) },
            AsRawHandle::as_raw_handle,
        ))?;
        let stdout = duplicate_inheritable(spec.stdout.as_ref().map_or_else(
            || unsafe { GetStdHandle(STD_OUTPUT_HANDLE) },
            AsRawHandle::as_raw_handle,
        ))?;
        let stderr = duplicate_inheritable(spec.stderr.as_ref().map_or_else(
            || unsafe { GetStdHandle(STD_ERROR_HANDLE) },
            AsRawHandle::as_raw_handle,
        ))?;
        let inherited = [stdin.raw(), stdout.raw(), stderr.raw()];
        let mut attributes = OwnedAttributeList::for_handles(&inherited)?;
        let mut startup = StartupInfoExW {
            startup_info: StartupInfoW {
                size: std::mem::size_of::<StartupInfoExW>() as u32,
                reserved: std::ptr::null_mut(),
                desktop: std::ptr::null_mut(),
                title: std::ptr::null_mut(),
                x: 0,
                y: 0,
                x_size: 0,
                y_size: 0,
                x_chars: 0,
                y_chars: 0,
                fill_attribute: 0,
                flags: STARTF_USESTDHANDLES,
                show_window: 0,
                reserved_2_size: 0,
                reserved_2: std::ptr::null_mut(),
                stdin: stdin.raw(),
                stdout: stdout.raw(),
                stderr: stderr.raw(),
            },
            attribute_list: attributes.raw(),
        };
        let mut process_info = ProcessInformation {
            process: std::ptr::null_mut(),
            thread: std::ptr::null_mut(),
            process_id: 0,
            thread_id: 0,
        };
        let application_name = wide_nul(spec.program.as_os_str(), "autonomous child program")?;
        let mut command_line = command_line(&spec.argv)?;
        let environment = environment_block(&spec.environment.variables)?;
        let current_directory = spec
            .current_dir
            .as_ref()
            .map(|path| wide_nul(path.as_os_str(), "autonomous child current directory"))
            .transpose()?
            .unwrap_or_default();
        // SAFETY: every pointer is null or points to a live, correctly terminated buffer.
        if unsafe {
            CreateProcessW(
                application_name.as_ptr(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
                environment.as_ptr().cast(),
                if current_directory.is_empty() {
                    std::ptr::null()
                } else {
                    current_directory.as_ptr()
                },
                (&mut startup as *mut StartupInfoExW).cast(),
                &mut process_info,
            )
        } == 0
        {
            return Err(last_error("create suspended autonomous child"));
        }
        let process = OwnedHandle::new(process_info.process, "own autonomous process handle")?;
        let thread = OwnedHandle::new(process_info.thread, "own autonomous thread handle")?;

        // SAFETY: both handles are live and owned throughout assignment.
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
            let assignment = std::io::Error::last_os_error();
            // SAFETY: the suspended process is still exclusively owned by this launch transaction.
            let terminated = unsafe { TerminateProcess(process.raw(), 1) };
            let cleanup = if terminated == 0 {
                format!(
                    "terminate suspended child failed: {}",
                    std::io::Error::last_os_error()
                )
            } else if unsafe { WaitForSingleObject(process.raw(), INFINITE) } != WAIT_OBJECT_0 {
                format!(
                    "wait for terminated suspended child failed: {}",
                    std::io::Error::last_os_error()
                )
            } else {
                "suspended child terminated and reaped".to_string()
            };
            return Err(format!(
                "assign suspended autonomous child to Job Object: {assignment}; cleanup: {cleanup}"
            ));
        }
        let creation_filetime = process_creation_filetime(process.raw()).inspect_err(|_| {
            // SAFETY: assignment succeeded, so terminating the Job Object covers the full tree.
            unsafe { TerminateJobObject(job.raw(), 1) };
        })?;
        // SAFETY: thread is the suspended primary thread returned by CreateProcessW.
        if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
            // SAFETY: the process is assigned, so this kills it before any child code runs.
            unsafe { TerminateJobObject(job.raw(), 1) };
            return Err(last_error("resume autonomous child primary thread"));
        }

        Ok(Self {
            job,
            process,
            thread: Some(thread),
            pid: process_info.process_id,
            creation_filetime,
            terminal: None,
            job_terminated: false,
        })
    }

    pub(in crate::commands::autonomous::executor_bridge::process_owner) fn id(&self) -> u32 {
        self.pid
    }

    pub(in crate::commands::autonomous::executor_bridge::process_owner) fn creation_filetime(
        &self,
    ) -> u64 {
        self.creation_filetime
    }

    pub(in crate::commands::autonomous::executor_bridge::process_owner) fn try_wait(
        &mut self,
    ) -> Result<Option<ExitStatus>, String> {
        if let Some(status) = self.terminal {
            return Ok(Some(status));
        }
        // SAFETY: process is a live owned process handle.
        match unsafe { WaitForSingleObject(self.process.raw(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => self.read_terminal().map(Some),
            _ => Err(last_error("poll autonomous Job Object process")),
        }
    }

    pub(in crate::commands::autonomous::executor_bridge::process_owner) fn wait(
        &mut self,
    ) -> Result<ExitStatus, String> {
        if let Some(status) = self.terminal {
            return Ok(status);
        }
        // SAFETY: process is a live owned process handle.
        if unsafe { WaitForSingleObject(self.process.raw(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error("wait for autonomous Job Object process"));
        }
        self.read_terminal()
    }

    pub(in crate::commands::autonomous::executor_bridge::process_owner) fn terminate(
        &mut self,
    ) -> Result<ExitStatus, String> {
        if !self.job_terminated {
            // SAFETY: job is the retained authority for this exact child tree, even after its
            // primary process exits while a descendant remains live.
            if unsafe { TerminateJobObject(self.job.raw(), 1) } == 0 {
                return Err(last_error("terminate autonomous Job Object"));
            }
            self.job_terminated = true;
        }
        self.wait()
    }

    fn read_terminal(&mut self) -> Result<ExitStatus, String> {
        let mut code = 0_u32;
        // SAFETY: process has signaled and code points to writable storage.
        if unsafe { GetExitCodeProcess(self.process.raw(), &mut code) } == 0 {
            return Err(last_error("read autonomous Job Object exit status"));
        }
        let status = ExitStatus::from_raw(code);
        self.terminal = Some(status);
        self.thread.take();
        Ok(status)
    }
}
