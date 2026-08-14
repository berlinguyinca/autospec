use super::PreparedLaunchSpec;
use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;

type Handle = *mut c_void;

const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const INFINITE: u32 = u32::MAX;
const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;
const STD_INPUT_HANDLE: u32 = -10_i32 as u32;
const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;

#[repr(C)]
#[derive(Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
struct StartupInfoW {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_chars: u32,
    y_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved_2_size: u16,
    reserved_2: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    attribute_list: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

#[repr(C)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<StartupInfoW>() == 104);
    assert!(std::mem::size_of::<StartupInfoExW>() == 112);
    assert!(std::mem::size_of::<ProcessInformation>() == 24);
    assert!(std::mem::size_of::<JobObjectBasicLimitInformation>() == 64);
    assert!(std::mem::size_of::<JobObjectExtendedLimitInformation>() == 144);
};

#[link(name = "Kernel32")]
unsafe extern "system" {
    fn CreateProcessW(
        application_name: *const u16,
        command_line: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit_handles: i32,
        creation_flags: u32,
        environment: *const c_void,
        current_directory: *const u16,
        startup_info: *mut StartupInfoW,
        process_information: *mut ProcessInformation,
    ) -> i32;
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, size: u32) -> i32;
    fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
    fn ResumeThread(thread: Handle) -> u32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
    fn TerminateJobObject(job: Handle, exit_code: u32) -> i32;
    fn TerminateProcess(process: Handle, exit_code: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetCurrentProcess() -> Handle;
    fn GetStdHandle(which: u32) -> Handle;
    fn DuplicateHandle(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        desired_access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
    fn InitializeProcThreadAttributeList(
        attribute_list: *mut c_void,
        attribute_count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        attribute_list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous_value: *mut c_void,
        return_size: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(attribute_list: *mut c_void);
}

struct OwnedHandle(Handle);

impl OwnedHandle {
    fn new(handle: Handle, operation: &str) -> Result<Self, String> {
        if handle.is_null() || handle as isize == -1 {
            Err(last_error(operation))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> Handle {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this RAII wrapper is constructed only for uniquely owned handles.
        unsafe { CloseHandle(self.0) };
    }
}

struct OwnedAttributeList {
    storage: Vec<usize>,
}

impl OwnedAttributeList {
    fn for_handles(handles: &[Handle]) -> Result<Self, String> {
        let mut bytes = 0_usize;
        // SAFETY: the documented sizing call accepts a null attribute list.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes) };
        if bytes == 0 {
            return Err(last_error("size autonomous child handle allowlist"));
        }
        let words = bytes.div_ceil(std::mem::size_of::<usize>());
        let mut list = Self {
            storage: vec![0; words],
        };
        // SAFETY: storage is pointer-aligned, writable, and at least the requested byte size.
        if unsafe { InitializeProcThreadAttributeList(list.raw(), 1, 0, &mut bytes) } == 0 {
            list.storage.clear();
            return Err(last_error("initialize autonomous child handle allowlist"));
        }
        // SAFETY: list is initialized and handles remains live through this call.
        if unsafe {
            UpdateProcThreadAttribute(
                list.raw(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error("set autonomous child handle allowlist"));
        }
        Ok(list)
    }

    fn raw(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for OwnedAttributeList {
    fn drop(&mut self) {
        if !self.storage.is_empty() {
            // SAFETY: nonempty storage contains an initialized attribute list after construction.
            unsafe { DeleteProcThreadAttributeList(self.raw()) };
        }
    }
}

pub(super) struct WindowsJobChild {
    job: OwnedHandle,
    process: OwnedHandle,
    thread: Option<OwnedHandle>,
    pid: u32,
    creation_filetime: u64,
    terminal: Option<ExitStatus>,
    job_terminated: bool,
}

impl WindowsJobChild {
    pub(super) fn spawn(spec: PreparedLaunchSpec) -> Result<Self, String> {
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

    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    pub(super) fn creation_filetime(&self) -> u64 {
        self.creation_filetime
    }

    pub(super) fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
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

    pub(super) fn wait(&mut self) -> Result<ExitStatus, String> {
        if let Some(status) = self.terminal {
            return Ok(status);
        }
        // SAFETY: process is a live owned process handle.
        if unsafe { WaitForSingleObject(self.process.raw(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error("wait for autonomous Job Object process"));
        }
        self.read_terminal()
    }

    pub(super) fn terminate(&mut self) -> Result<ExitStatus, String> {
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

fn duplicate_inheritable(source: RawHandle) -> Result<OwnedHandle, String> {
    let source = source.cast::<c_void>();
    if source.is_null() || source as isize == -1 {
        return Err("autonomous child standard handle is unavailable".to_string());
    }
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: source is borrowed for this call and duplicate points to writable handle storage.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(last_error("duplicate autonomous child standard handle"));
    }
    OwnedHandle::new(duplicate, "own autonomous child standard handle")
}

fn process_creation_filetime(process: Handle) -> Result<u64, String> {
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    // SAFETY: process is live and every FILETIME pointer is writable.
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(last_error("capture autonomous child creation FILETIME"));
    }
    let value = ((creation.high as u64) << 32) | creation.low as u64;
    if value == 0 {
        Err("autonomous child creation FILETIME is empty".to_string())
    } else {
        Ok(value)
    }
}

fn command_line(argv: &[std::ffi::OsString]) -> Result<Vec<u16>, String> {
    let Some((argv_zero, arguments)) = argv.split_first() else {
        return Err("autonomous child argv is empty".to_string());
    };
    let mut encoded = Vec::new();
    push_quoted(&mut encoded, argv_zero)?;
    for argument in arguments {
        encoded.push(b' ' as u16);
        push_quoted(&mut encoded, argument)?;
    }
    encoded.push(0);
    Ok(encoded)
}

fn push_quoted(output: &mut Vec<u16>, value: &OsStr) -> Result<(), String> {
    let wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err("autonomous child command contains an embedded NUL".to_string());
    }
    let needs_quotes = wide.is_empty()
        || wide
            .iter()
            .any(|unit| *unit == b' ' as u16 || *unit == b'\t' as u16 || *unit == b'"' as u16);
    if !needs_quotes {
        output.extend(wide);
        return Ok(());
    }
    output.push(b'"' as u16);
    let mut slashes = 0;
    for unit in wide {
        if unit == b'\\' as u16 {
            slashes += 1;
        } else {
            if unit == b'"' as u16 {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            slashes = 0;
            output.push(unit);
        }
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(())
}

fn environment_block(
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Result<Vec<u16>, String> {
    let mut environment = environment.to_vec();
    environment.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (key, value) in environment {
        block.extend(wide(&key, "autonomous child environment key")?);
        block.push(b'=' as u16);
        block.extend(wide(&value, "autonomous child environment value")?);
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn wide_nul(value: &OsStr, label: &str) -> Result<Vec<u16>, String> {
    let mut encoded = wide(value, label)?;
    encoded.push(0);
    Ok(encoded)
}

fn wide(value: &OsStr, label: &str) -> Result<Vec<u16>, String> {
    let encoded: Vec<u16> = value.encode_wide().collect();
    if encoded.contains(&0) {
        Err(format!("{label} contains an embedded NUL"))
    } else {
        Ok(encoded)
    }
}

fn last_error(operation: &str) -> String {
    format!("{operation}: {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    fn rendered(argv: &[&str]) -> String {
        let argv = argv.iter().map(OsString::from).collect::<Vec<_>>();
        let encoded = command_line(&argv).expect("encode Windows command line");
        String::from_utf16(&encoded[..encoded.len() - 1]).expect("decode command line")
    }

    #[test]
    fn command_line_quotes_windows_argv_edge_cases() {
        let cases = [
            (vec![""], "\"\""),
            (vec!["a b"], "\"a b\""),
            (vec!["a\\b"], "a\\b"),
            (vec!["a\"b"], "\"a\\\"b\""),
            (vec!["a b\\"], "\"a b\\\\\""),
            (vec!["a\\\"b"], "\"a\\\\\\\"b\""),
        ];
        for (argv, expected) in cases {
            assert_eq!(rendered(&argv), expected, "argv: {argv:?}");
        }
    }

    #[test]
    fn command_line_rejects_embedded_nul() {
        let program = OsString::from_wide(&[b'a' as u16, 0, b'b' as u16]);
        assert_eq!(
            command_line(&[program]).unwrap_err(),
            "autonomous child command contains an embedded NUL"
        );
    }
}
