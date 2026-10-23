//! forge-tray: the pipeline in the Windows taskbar.
//!
//!   forge-tray            tray icon; its menu lists every group and station from forge's /api/stations
//!   forge-tray --print    the same menu as text (any platform; no tray)
//!
//! Menu: Open workspace (forge's page) · one submenu per group (family) · per station its own .bat buttons and
//! "Open folder" · Refresh · Quit. A click runs exactly the .bat the feed listed, in its own console window, the same
//! as double-clicking it. The tray never builds a command line of its own and never calls a model.
//! Forge's address: FORGE_URL (default http://127.0.0.1:8790).
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

#[cfg(windows)]
mod tray {
    use crate::model::{self, Group};
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
            "Open workspace",
            true,
            Some(Action::Open(format!("{base}/"))),
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
        let (menu, mut actions) = build_menu(base)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("Pipeline stations")
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
                        let (m, a) = build_menu(base)?;
                        tray.set_menu(Some(Box::new(m)));
                        actions = a;
                    }
                    Some(Action::Quit) => return Ok(()),
                    None => {}
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
