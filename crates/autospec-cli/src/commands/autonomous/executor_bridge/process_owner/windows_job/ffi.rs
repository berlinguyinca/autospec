use std::ffi::c_void;
use std::os::windows::io::RawHandle;

mod child;
pub(in crate::commands::autonomous::executor_bridge::process_owner) use child::WindowsJobChild;

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

pub(super) fn last_error(operation: &str) -> String {
    format!("{operation}: {}", std::io::Error::last_os_error())
}
