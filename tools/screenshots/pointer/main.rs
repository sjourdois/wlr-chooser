// shots-pointer — minimal virtual-pointer + virtual-keyboard injector for the
// screenshot generator. Creates a zwlr_virtual_pointer_v1 (giving the input-less
// headless seat a pointer) and a zwp_virtual_keyboard_v1 (a keyboard that can
// HOLD keys, which wtype cannot — needed for wlr-draw's Shift-held spotlight).
// Reads commands from stdin, one per line, keeping both devices alive:
//
//   abs X Y W H      absolute pointer motion to (X,Y) within a W×H space
//   btn CODE STATE   pointer button (l/r/m or evdev number), STATE 1=down 0=up
//   scroll AXIS N    discrete scroll, AXIS v|h, N notches (sign = direction)
//   key CODE STATE   keyboard key by evdev code (Shift=42, Ctrl=29, Space=57…),
//                    STATE 1=down 0=up — held until released
//   quit             exit

use std::io::{self, BufRead, Seek, SeekFrom, Write};
use std::os::fd::AsFd;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use xkbcommon::xkb;

struct State;

macro_rules! ignore_events {
    ($($t:ty),* $(,)?) => { $(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(),
                     _: &Connection, _: &QueueHandle<Self>) {}
        }
    )* };
}
ignore_events!(
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1,
    ZwpVirtualKeyboardManagerV1,
    ZwpVirtualKeyboardV1,
    wl_seat::WlSeat,
);
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
             _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

fn btn_code(s: &str) -> u32 {
    match s {
        "l" | "left" => 0x110,
        "r" | "right" => 0x111,
        "m" | "middle" => 0x112,
        other => other.parse().unwrap_or(0x110),
    }
}

// evdev keycode -> xkb modifier mask bit, for the keys we hold.
fn mod_mask(code: u32) -> u32 {
    match code {
        42 | 54 => 1, // Shift_L / Shift_R
        29 | 97 => 4, // Ctrl_L / Ctrl_R
        56 | 100 => 8, // Alt_L / Alt_R
        _ => 0,
    }
}

fn main() {
    let conn = Connection::connect_to_env().expect("connect to nested wayland");
    let (globals, mut queue) = registry_queue_init::<State>(&conn).expect("registry init");
    let qh = queue.handle();
    let mut state = State;

    let ptr_mgr: ZwlrVirtualPointerManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .expect("compositor lacks zwlr_virtual_pointer_manager_v1");
    let seat: Option<wl_seat::WlSeat> = globals.bind(&qh, 1..=8, ()).ok();
    let pointer = ptr_mgr.create_virtual_pointer(seat.as_ref(), &qh, ());

    // Virtual keyboard with a standard US keymap (optional — pointer-only if absent).
    let keyboard: Option<ZwpVirtualKeyboardV1> = globals
        .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
        .ok()
        .map(|kmgr| {
            let kbd = kmgr.create_virtual_keyboard(seat.as_ref().expect("seat"), &qh, ());
            let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            let keymap = xkb::Keymap::new_from_names(
                &ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS,
            )
            .expect("compile keymap");
            let s = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
            let mut f = tempfile::tempfile().expect("keymap tmpfile");
            f.write_all(s.as_bytes()).unwrap();
            f.write_all(&[0]).unwrap();
            f.flush().unwrap();
            f.seek(SeekFrom::Start(0)).unwrap();
            kbd.keymap(1 /* xkb v1 */, f.as_fd(), (s.len() + 1) as u32);
            queue.roundtrip(&mut state).unwrap(); // let the compositor read the fd
            kbd
        });

    queue.roundtrip(&mut state).unwrap();

    let mut t: u32 = 1;
    let mut tick = || {
        let v = t;
        t = t.wrapping_add(16);
        v
    };
    let mut mods: u32 = 0;

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let mut it = line.split_whitespace();
        match it.next() {
            Some("abs") => {
                let x: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let y: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let w: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(1920);
                let h: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(1080);
                pointer.motion_absolute(tick(), x, y, w, h);
                pointer.frame();
            }
            Some("btn") => {
                let code = it.next().map(btn_code).unwrap_or(0x110);
                let down = it.next().map(|s| s != "0").unwrap_or(true);
                let st = if down {
                    wl_pointer::ButtonState::Pressed
                } else {
                    wl_pointer::ButtonState::Released
                };
                pointer.button(tick(), code, st);
                pointer.frame();
            }
            Some("scroll") => {
                let axis = match it.next() {
                    Some("h") => wl_pointer::Axis::HorizontalScroll,
                    _ => wl_pointer::Axis::VerticalScroll,
                };
                let n: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
                let ti = tick();
                pointer.axis_source(wl_pointer::AxisSource::Wheel);
                pointer.axis_discrete(ti, axis, n * 15.0, n as i32);
                pointer.frame();
            }
            Some("key") => {
                if let Some(kbd) = &keyboard {
                    let code: u32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                    let down = it.next().map(|s| s != "0").unwrap_or(true);
                    kbd.key(tick(), code, down as u32);
                    let m = mod_mask(code);
                    if m != 0 {
                        if down {
                            mods |= m;
                        } else {
                            mods &= !m;
                        }
                        kbd.modifiers(mods, 0, 0, 0);
                    }
                }
            }
            Some("quit") => break,
            _ => {}
        }
        let _ = conn.flush();
        let _ = queue.roundtrip(&mut state);
    }

    pointer.destroy();
    let _ = conn.flush();
}
