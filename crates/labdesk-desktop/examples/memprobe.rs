// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Dev tool: prints how much memory the UI uses on each Settings tab with the full 288-test list.
use labdesk_core::{Actor, Role, Store};
use labdesk_desktop::{app, AppWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError, Rgb8Pixel, SharedPixelBuffer};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;
thread_local! { static NOW: Cell<u64> = const { Cell::new(0) }; }
struct P(Rc<MinimalSoftwareWindow>);
impl Platform for P {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> { Ok(self.0.clone()) }
    fn duration_since_start(&self) -> Duration { Duration::from_millis(NOW.with(|n| n.get())) }
}
const W: u32 = 1920; const H: u32 = 1080;
fn advance(ms: u64) { for _ in 0..(ms / 50).max(1) { NOW.with(|n| n.set(n.get() + 50)); slint::platform::update_timers_and_animations(); } }
fn rss() -> u64 { std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with("VmRSS")).unwrap().split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024 }
fn frame(win: &Rc<MinimalSoftwareWindow>, buf: &mut SharedPixelBuffer<Rgb8Pixel>) { win.request_redraw(); win.draw_if_needed(|r| { r.render(buf.make_mut_slice(), W as usize); }); }
fn main() {
    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(P(win.clone()))).unwrap();
    win.set_size(PhysicalSize::new(W, H));
    let mut store = Store::open_in_memory().unwrap();
    let a = store.create_first_admin("mutaib", "Syed Mutaib", "secret1").unwrap();
    store.import_catalogue(&a.actor(), app::STANDARD_TESTS).unwrap();
    println!("RSS with data loaded, no UI: {} MB", rss());
    let ui = AppWindow::new().unwrap();
    ui.show().unwrap();
    let opts = app::Options { docs_dir: std::env::temp_dir().join("md"), open_files: false, use_sessions: false, ..app::Options::new() };
    let _ctl = app::wire(&ui, store, opts);
    let mut buf = SharedPixelBuffer::<Rgb8Pixel>::new(W, H);
    ui.set_l_user("mutaib".into()); ui.set_l_pass("secret1".into()); ui.invoke_sign_in();
    advance(1000); frame(&win, &mut buf); advance(500); frame(&win, &mut buf);
    println!("RSS home: {} MB", rss());
    for tab in 0..9 {
        ui.set_s_tab(tab); ui.invoke_navigate(3);
        for _ in 0..4 { advance(300); frame(&win, &mut buf); }
        println!("RSS settings tab {tab}: {} MB", rss());
    }
    ui.invoke_navigate(0); for _ in 0..4 { advance(300); frame(&win, &mut buf); }
    println!("RSS back home: {} MB", rss());
}
