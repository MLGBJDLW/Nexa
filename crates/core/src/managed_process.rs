//! A pipe-connected background process whose descendants share its lifetime.
//! This is lifecycle containment, not a filesystem or network sandbox.
use tokio::process::{Child, Command};

pub struct ProcessTree {
    #[cfg(windows)]
    job: isize,
    #[cfg(unix)]
    group: i32,
}

pub fn spawn(command: &mut Command) -> Result<(Child, ProcessTree), std::io::Error> {
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    match attach(&mut child) {
        Ok(tree) => Ok((child, tree)),
        Err(error) => {
            let _ = child.start_kill();
            Err(error)
        }
    }
}

#[cfg(windows)]
fn attach(child: &mut Child) -> Result<ProcessTree, std::io::Error> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::*,
    };
    let process = child
        .raw_handle()
        .ok_or_else(|| std::io::Error::other("Missing child handle"))?;
    unsafe {
        let job = CreateJobObjectW(None, None).map_err(std::io::Error::other)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&info) as u32,
        )
        .and_then(|()| AssignProcessToJobObject(job, HANDLE(process as *mut _)));
        if let Err(error) = result {
            let _ = CloseHandle(job);
            return Err(std::io::Error::other(error));
        }
        Ok(ProcessTree {
            job: job.0 as isize,
        })
    }
}

#[cfg(unix)]
fn attach(child: &mut Child) -> Result<ProcessTree, std::io::Error> {
    Ok(ProcessTree {
        group: child
            .id()
            .ok_or_else(|| std::io::Error::other("Missing child id"))? as i32,
    })
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(
                self.job as *mut _,
            ));
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
    }
}
