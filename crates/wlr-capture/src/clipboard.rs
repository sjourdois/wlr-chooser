//! Native Wayland clipboard copy via `zwlr_data_control_v1` (the wlroots
//! clipboard-manager protocol, the same one `wl-copy` and `grim` use).
//!
//! [`serve`] offers a single blob under one MIME type and answers paste requests
//! until another client takes ownership of the selection. The Wayland clipboard is
//! *pull*-based: the data lives in the providing client, which must stay alive and
//! write the bytes whenever a consumer pastes. So this blocks for the lifetime of
//! the selection — callers that want their shell back should run it detached in the
//! background (see `wlr-shot`'s clipboard daemon).
//!
//! Only the `data-control` path is implemented: it needs no surface or input focus,
//! which suits a headless capture tool. The classic `wl_data_device` fallback
//! requires a focused surface and an input serial, so it is deliberately omitted —
//! every wlroots compositor we target exposes `data-control`.

use crate::error::{Context, Result};
use std::fs::File;
use std::io::Write;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry::WlRegistry, wl_seat::WlSeat},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

struct ClipState {
    /// The MIME type we advertise; paste requests for anything else get an empty fd.
    mime: String,
    /// The bytes to hand a consumer that pastes our offer.
    data: Vec<u8>,
    /// Set once another client took the selection (our offer is gone): time to exit.
    done: bool,
}

/// Put `data` on the Wayland clipboard, advertised as `mime`, and serve paste
/// requests until the selection is replaced. **Blocks** for the lifetime of the
/// selection (see the module docs); run it detached if you need the caller back.
pub fn serve(mime: &str, data: Vec<u8>) -> Result<()> {
    let conn = Connection::connect_to_env().context("Wayland connection")?;
    let (globals, mut queue) =
        registry_queue_init::<ClipState>(&conn).context("registre Wayland")?;
    let qh = queue.handle();

    let mgr: ZwlrDataControlManagerV1 = globals.bind(&qh, 1..=2, ()).context(
        "zwlr_data_control_manager_v1 missing (compositor has no wlroots clipboard support)",
    )?;
    let seat: WlSeat = globals.bind(&qh, 1..=8, ()).context("wl_seat missing")?;

    // Publish the selection: a source that offers our MIME type, set on the device.
    let device = mgr.get_data_device(&seat, &qh, ());
    let source = mgr.create_data_source(&qh, ());
    source.offer(mime.to_string());
    device.set_selection(Some(&source));

    let mut state = ClipState {
        mime: mime.to_string(),
        data,
        done: false,
    };
    queue.roundtrip(&mut state).context("set_selection")?;

    // Serve paste requests until our offer is cancelled (another client took over).
    while !state.done {
        queue
            .blocking_dispatch(&mut state)
            .context("clipboard event loop")?;
    }
    Ok(())
}

/// Detach a background daemon that keeps a clipboard selection alive past the caller.
///
/// The Wayland clipboard is pull-based, so [`serve`] must outlive the process that
/// set the selection. This re-execs the current binary with `args` — which must
/// route to a (typically hidden) subcommand that reads the blob from stdin and calls
/// [`serve`] — in its own session (`setsid`, so the parent's terminal closing sends
/// no SIGHUP), pipes `bytes` to its stdin, and returns immediately.
pub fn spawn_detached(bytes: &[u8], args: &[&str]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().context("current executable path")?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // setsid() so the daemon survives this process exiting and its controlling
    // terminal closing.
    unsafe {
        cmd.pre_exec(|| rustix::process::setsid().map(|_| ()).map_err(Into::into));
    }
    let mut child = cmd.spawn().context("spawning clipboard daemon")?;
    child
        .stdin
        .take()
        .context("daemon stdin")?
        .write_all(bytes)
        .context("sending data to daemon")?;
    // Don't wait: the daemon keeps serving the selection in the background.
    Ok(())
}

impl Dispatch<WlRegistry, GlobalListContents> for ClipState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for ClipState {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_data_control_source_v1::Event;
        match event {
            // A consumer is pasting: write our bytes into the pipe it handed us.
            // Dropping the `File` closes the fd, which is the EOF the reader waits
            // for. A broken pipe (consumer gave up) is harmless — ignore it.
            Event::Send { mime_type, fd } => {
                if mime_type == state.mime {
                    let _ = File::from(fd).write_all(&state.data);
                }
                // For a non-matching MIME, `fd` drops here: an empty reply.
            }
            // Our selection was replaced by another client: nothing left to serve.
            Event::Cancelled => state.done = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        _: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // We only publish a selection; incoming offers and selection changes from
        // other clients are of no interest here.
    }

    // The device introduces every advertised offer as a new object; register the
    // child so the library can route it (its events are then ignored, below).
    event_created_child!(ClipState, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

delegate_noop!(ClipState: ignore ZwlrDataControlManagerV1);
delegate_noop!(ClipState: ignore ZwlrDataControlOfferV1);
delegate_noop!(ClipState: ignore WlSeat);
