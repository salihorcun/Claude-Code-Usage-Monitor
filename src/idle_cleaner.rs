use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

const IDLE_AFTER: Duration = Duration::from_secs(30 * 60);
const GRACE_AFTER: Duration = Duration::from_secs(5 * 60);

const MANAGED_APPS: &[&str] = &[
    "discord.exe",
    "steam.exe",
    "epicgameslauncher.exe",
    "spotify.exe",
    "teams.exe",
    "ms-teams.exe",
    "telegram.exe",
    "whatsapp.exe",
];

const PROTECTED_APPS: &[&str] = &[
    "3dsmax.exe",
    "coronaimagebatch.exe",
    "coronarenderer.exe",
    "photoshop.exe",
    "explorer.exe",
    "dwm.exe",
    "system",
    "claude-code-usage-monitor.exe",
];

#[derive(Debug)]
pub struct IdleCleaner {
    last_foreground: HashMap<String, Instant>,
    warned_at: HashMap<String, Instant>,
    idle_count: u32,
    running_count: u32,
}

impl Default for IdleCleaner {
    fn default() -> Self {
        Self {
            last_foreground: HashMap::new(),
            warned_at: HashMap::new(),
            idle_count: 0,
            running_count: 0,
        }
    }
}

impl IdleCleaner {
    pub fn idle_count(&self) -> u32 {
        self.idle_count
    }

    pub fn running_count(&self) -> u32 {
        self.running_count
    }

    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let running = running_processes();
        let foreground = foreground_process_name(&running);
        let managed: HashSet<&str> = MANAGED_APPS.iter().copied().collect();
        let protected: HashSet<&str> = PROTECTED_APPS.iter().copied().collect();

        if let Some(name) = foreground.as_deref() {
            if managed.contains(name) {
                self.last_foreground.insert(name.to_string(), now);
                self.warned_at.remove(name);
            }
        }

        self.last_foreground
            .retain(|name, _| running.values().any(|running_name| running_name == name));
        self.warned_at
            .retain(|name, _| running.values().any(|running_name| running_name == name));

        let mut idle_count = 0_u32;
        let mut running_count = 0_u32;

        for &name in MANAGED_APPS {
            if protected.contains(name) {
                continue;
            }
            if !running.values().any(|running_name| running_name == name) {
                continue;
            }

            running_count += 1;
            let last = self.last_foreground.entry(name.to_string()).or_insert(now);
            let idle_for = now.saturating_duration_since(*last);
            if idle_for < IDLE_AFTER {
                self.warned_at.remove(name);
                continue;
            }

            idle_count += 1;
            let warned = self.warned_at.entry(name.to_string()).or_insert(now);
            if now.saturating_duration_since(*warned) >= GRACE_AFTER {
                close_gracefully(name);
                self.warned_at.remove(name);
                self.last_foreground.insert(name.to_string(), now);
            }
        }

        let changed = self.idle_count != idle_count || self.running_count != running_count;
        self.idle_count = idle_count;
        self.running_count = running_count;
        changed
    }
}

fn running_processes() -> HashMap<u32, String> {
    let mut result = HashMap::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return result;
    };

    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_ascii_lowercase();
            result.insert(entry.th32ProcessID, name);

            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };
    result
}

fn foreground_process_name(running: &HashMap<u32, String>) -> Option<String> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0_u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    running.get(&pid).cloned()
}

fn close_gracefully(exe_name: &str) {
    // taskkill without /F asks GUI applications to terminate normally.
    // It intentionally avoids force-killing so unsaved work can still prompt.
    let _ = Command::new("taskkill")
        .args(["/IM", exe_name])
        .output();
}
