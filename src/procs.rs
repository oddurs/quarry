//! The `sysinfo` process source.

use std::path::Path;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::source::{ProcInfo, ProcessSource};

pub struct SysProcesses {
    sys: System,
}

impl SysProcesses {
    pub fn new() -> Self {
        Self { sys: System::new() }
    }
}

impl Default for SysProcesses {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessSource for SysProcesses {
    fn refresh(&mut self, pids: &[u32]) {
        let pids: Vec<Pid> = pids.iter().map(|p| Pid::from_u32(*p)).collect();
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing()
                .with_cmd(UpdateKind::Always)
                .with_exe(UpdateKind::Always)
                .with_cwd(UpdateKind::Always)
                .with_user(UpdateKind::Always)
                .with_cpu()
                .with_memory(),
        );
    }

    fn info(&self, pid: u32) -> Option<ProcInfo> {
        let p = self.sys.process(Pid::from_u32(pid))?;
        Some(ProcInfo {
            cmdline: p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
            name: p.name().to_string_lossy().to_string(),
            exe: p.exe().map(Path::to_path_buf),
            cwd: p.cwd().map(Path::to_path_buf),
            ppid: p.parent().map(|p| p.as_u32()),
            started_at: p.start_time(),
            cpu: p.cpu_usage(),
            mem: p.memory(),
        })
    }

    fn describe(&self) -> String {
        "sysinfo".to_string()
    }
}
