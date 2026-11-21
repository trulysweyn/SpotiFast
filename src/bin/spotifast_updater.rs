#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;
use std::ptr;

use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn move_replace(source: &Path, destination: &Path) -> Result<(), String> {
    let source = wide(source.as_os_str());
    let destination = wide(destination.as_os_str());
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

fn show_error(message: &str) {
    let title = wide(OsStr::new("SpotiFast update"));
    let message = wide(OsStr::new(message));
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn wait_for_parent(pid: u32) -> Result<(), String> {
    let process = unsafe { OpenProcess(0x0010_0000, 0, pid) };
    if process.is_null() {
        return Ok(());
    }
    let wait_result = unsafe { WaitForSingleObject(process, 120_000) };
    unsafe { windows_sys::Win32::Foundation::CloseHandle(process) };
    if wait_result == windows_sys::Win32::Foundation::WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err("SpotiFast is still open. Close it and try the update again.".to_string())
    }
}

fn apply_update(arguments: &[String]) -> Result<(), String> {
    if arguments.len() != 4 {
        return Err(
            "The update could not start because its arguments were incomplete.".to_string(),
        );
    }
    let target = Path::new(&arguments[0]);
    let staged = Path::new(&arguments[1]);
    let rollback = Path::new(&arguments[2]);
    let parent_pid = arguments[3]
        .parse::<u32>()
        .map_err(|_| "The update could not identify the running app.".to_string())?;
    if !target.is_file() || !staged.is_file() {
        return Err("The new SpotiFast file or installed app could not be found.".to_string());
    }
    wait_for_parent(parent_pid)?;

    let backup_pending = rollback.with_extension("pending.exe");
    let replacement = target.with_file_name("SpotiFast-update.tmp.exe");
    std::fs::copy(target, &backup_pending)
        .map_err(|error| format!("Could not save the current app for rollback: {error}"))?;
    move_replace(&backup_pending, rollback)
        .map_err(|error| format!("Could not save the rollback version: {error}"))?;

    std::fs::copy(staged, &replacement).map_err(|error| {
        format!("Could not prepare the new app beside the installed app: {error}")
    })?;
    move_replace(&replacement, target)
        .map_err(|error| format!("Could not install the new app: {error}"))?;
    std::fs::remove_file(staged).ok();

    Command::new(target).spawn().map_err(|error| {
        format!("The update installed, but SpotiFast could not be restarted: {error}")
    })?;
    Ok(())
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = apply_update(&arguments) {
        show_error(&error);
    }
}
