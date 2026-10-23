//! forge-tray: the pipeline in the Windows taskbar.
//!
//!   forge-tray            tray icon; starts forge if it isn't running; its menu lists every group and station
//!   forge-tray --print    the same menu as text (any platform; no tray, starts nothing)
//!
//! Menu: Open home (every API source on one page) · Open workspace (Kimi's page) · one submenu per group · per station
//! its own buttons (.bat runs in its own console, .html opens in the browser) and "Open folder" · Troubleshoot ·
//! Refresh · Quit. A click starts exactly the file the feed listed, the same as double-clicking it. The tray never
//! builds a command line of its own and never calls a model.
//!
//! No port to think about: when nothing answers at FORGE_URL (default http://127.0.0.1:8790), the tray starts the
//! forge.exe next to it, hidden (`forge --home <its folder> serve`, log in LOGS\forge-serve.log), and stops it again on
//! Quit. A forge you started yourself is used as it is and left running.
//! On Windows it is a GUI program (no console window); `--print` is for checking the menu elsewhere.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod model;

fn main() -> anyhow::Result<()> {
    let base = model::forge_url();
    if std::env::args().any(|a| a == "--print") || !cfg!(windows) {
        match model::fetch(&base) {
            Ok(feed) => print!("{}", model::render(&model::build(&feed))),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
        if !cfg!(windows) {
            eprintln!("(the tray itself runs on Windows; this is its menu)");
        }
        return Ok(());
    }
    #[cfg(windows)]
    tray::run(&base)?;
    Ok(())
}

/// The folder this program lives in (the install folder: forge.exe, config, LOGS, TROUBLESHOOT.bat).
#[cfg_attr(not(windows), allow(dead_code))]
fn install_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| ".".into())
}

/// Start the forge next to this program when nothing answers at `base`. Returns the child if this call started it.
#[cfg_attr(not(windows), allow(dead_code))]
fn start_forge(base: &str) -> Option<std::process::Child> {
    if model::alive(base) {
        return None;
    }
    let dir = install_dir();
    let exe = dir.join(if cfg!(windows) { "forge.exe" } else { "forge" });
    if !exe.is_file() {
        return None;
    }
    let port = base
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(8790);
    let _ = std::fs::create_dir_all(dir.join("LOGS"));
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("LOGS").join("forge-serve.log"))
        .ok();
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--home")
        .arg(&dir)
        .args(["serve", "--port", &port.to_string()])
        .current_dir(&dir)
        .stdin(std::process::Stdio::null());
    if let Some(l) = log {
        if let Ok(l2) = l.try_clone() {
            cmd.stdout(l2);
        }
        cmd.stderr(l);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = cmd.spawn().ok()?;
    for _ in 0..80 {
        if model::alive(base) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Some(child)
}

#[cfg(windows)]
mod tray {
    use crate::model::{self, Group};
    use crate::{install_dir, start_forge};
    use std::collections::HashMap;
    use std::process::Command;
    use std::time::Duration;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{Icon, TrayIconBuilder};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    enum Action {
        Open(String),
        Run(String),
        Refresh,
        Quit,
    }

    /// The menu and, for every item id, what it does. Only paths from the feed end up in `Action::Run`.
    fn build_menu(base: &str) -> anyhow::Result<(Menu, HashMap<String, Action>)> {
        let menu = Menu::new();
        let mut actions = HashMap::new();
        let mut n = 0usize;
        let mut item = |text: &str,
                        enabled: bool,
                        action: Option<Action>,
                        actions: &mut HashMap<String, Action>| {
            n += 1;
            let id = format!("i{n}");
            if let Some(a) = action {
                actions.insert(id.clone(), a);
            }
            MenuItem::with_id(id, text, enabled, None)
        };
        menu.append(&item(
            "Open home",
            true,
            Some(Action::Open(format!("{base}/"))),
            &mut actions,
        ))?;
        menu.append(&item(
            "Open workspace",
            true,
            Some(Action::Open(format!("{base}/workspace"))),
            &mut actions,
        ))?;
        menu.append(&PredefinedMenuItem::separator())?;
        match model::fetch(base) {
            Ok(feed) => {
                let groups: Vec<Group> = model::build(&feed);
                for g in &groups {
                    let sub = Submenu::new(&g.name, true);
                    for s in &g.stations {
                        let st = Submenu::new(&s.label, true);
                        for b in &s.buttons {
                            st.append(&item(
                                &b.name,
                                true,
                                Some(Action::Run(b.path.clone())),
                                &mut actions,
                            ))?;
                        }
                        if !s.folder.is_empty() {
                            st.append(&PredefinedMenuItem::separator())?;
                            st.append(&item(
                                "Open folder",
                                true,
                                Some(Action::Open(s.folder.clone())),
                                &mut actions,
                            ))?;
                        }
                        sub.append(&st)?;
                    }
                    menu.append(&sub)?;
                }
            }
            Err(e) => {
                let text: String = e.to_string().chars().take(120).collect();
                menu.append(&item(&text, false, None, &mut actions))?;
            }
        }
        menu.append(&PredefinedMenuItem::separator())?;
        let doctor = install_dir().join("TROUBLESHOOT.bat");
        if doctor.is_file() {
            menu.append(&item(
                "Troubleshoot",
                true,
                Some(Action::Run(doctor.display().to_string())),
                &mut actions,
            ))?;
        }
        menu.append(&item("Refresh", true, Some(Action::Refresh), &mut actions))?;
        menu.append(&item("Quit", true, Some(Action::Quit), &mut actions))?;
        Ok((menu, actions))
    }

    fn icon() -> anyhow::Result<Icon> {
        const S: u32 = 32;
        let mut rgba = Vec::with_capacity((S * S * 4) as usize);
        for y in 0..S {
            for x in 0..S {
                let edge = x < 3 || y < 3 || x >= S - 3 || y >= S - 3;
                rgba.extend_from_slice(if edge {
                    &[200, 220, 255, 255]
                } else {
                    &[30, 80, 160, 255]
                });
            }
        }
        Ok(Icon::from_rgba(rgba, S, S)?)
    }

    /// `start "" <target>`: a .bat opens in its own console (same as a double-click), a folder in Explorer, a URL in the browser.
    fn open(target: &str) {
        let _ = Command::new("cmd")
            .args(["/C", "start", "", target])
            .spawn();
    }

    pub fn run(base: &str) -> anyhow::Result<()> {
        let mut started = start_forge(base);
        let (menu, mut actions) = build_menu(base)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("API home: pipeline stations")
            .with_icon(icon()?)
            .with_menu(Box::new(menu))
            .build()?;
        let events = MenuEvent::receiver();
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        loop {
            // The tray needs a Win32 message pump on this thread.
            unsafe {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            while let Ok(ev) = events.try_recv() {
                match actions.get(ev.id.as_ref()) {
                    Some(Action::Run(path)) | Some(Action::Open(path)) => open(path),
                    Some(Action::Refresh) => {
                        if started.is_none() {
                            started = start_forge(base);
                        }
                        let (m, a) = build_menu(base)?;
                        tray.set_menu(Some(Box::new(m)));
                        actions = a;
                    }
                    Some(Action::Quit) => {
                        // Stop only the forge this tray started; one you started yourself keeps running.
                        if let Some(mut c) = started.take() {
                            let _ = c.kill();
                        }
                        return Ok(());
                    }
                    None => {}
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
