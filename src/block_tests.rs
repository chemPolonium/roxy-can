//! Replay-block unit tests: queue loading, filtering, and emission
//! against a written ASC log.

use super::ReplayBlock;
use crate::can::frame::{CanFrame, Direction, FrameFlags, MAX_CAN_FD_LEN};
use crate::dbc::load_dbc_str;
use crate::log::AscWriter;

/// Writes a fixture log: 0x100 at 0/10/20 ms, one 0x200 at 10 ms, one
/// extended 0x200001 at 30 ms. One file per test -- the suite runs in
/// parallel threads, and a shared fixture races with its own rewrites.
fn write_log(name: &str) -> String {
    let path = std::env::temp_dir().join(name);
    let mut w = AscWriter::new(&path.to_string_lossy()).unwrap();
    let mut frame = |t_us: u64, id: u32, extended: bool, b0: u8| {
        let mut f = CanFrame {
            t_us,
            channel: 0,
            id,
            extended,
            len: 2,
            data: [0; MAX_CAN_FD_LEN],
            dir: Direction::Rx,
            flags: FrameFlags::NONE,
        };
        f.data[0] = b0;
        w.write(&f).unwrap();
    };
    frame(0, 0x100, false, 0x11);
    frame(10_000, 0x100, false, 0x33);
    frame(10_000, 0x200, false, 0x55);
    frame(20_000, 0x100, false, 0x66);
    frame(30_000, 0x200001, true, 0x88);
    w.finish().unwrap();
    path.to_string_lossy().into_owned()
}

fn block(path: &str, node: Option<&str>, ids: Vec<(u32, bool)>) -> ReplayBlock {
    ReplayBlock::new(
        1,
        "blk".to_string(),
        0,
        path.to_string(),
        node.map(String::from),
        None,
        ids,
        true,
    )
}

/// The id filter keeps only the frames it names, timestamps renormalize
/// so the first kept frame is due at zero, and the recorded spacing
/// survives.
#[test]
fn an_id_filtered_block_keeps_only_its_frames() {
    let path = write_log("roxy_can_block_id.asc");
    let mut b = block(&path, None, vec![(0x100, false)]);
    b.load_queue(None, 0);
    assert_eq!(b.last_error, None, "the log loads");
    assert_eq!(b.queue.len(), 3, "the three 0x100 frames");

    let mut out = Vec::new();
    b.poll(0, &mut out, 100);
    assert_eq!(out.len(), 1, "the first frame is due at zero");
    assert_eq!(out[0].data[0], 0x11);
    let mut out = Vec::new();
    b.poll(9_999, &mut out, 100);
    assert!(out.is_empty(), "not due yet");
    b.poll(20_000, &mut out, 100);
    assert_eq!(out.len(), 2, "the remaining frames flowed with the clock");
    assert_eq!(out[0].data[0], 0x33, "in log order");
    assert_eq!(out[1].data[0], 0x66);
}

/// A node filter resolves against the bus's database: only the frames
/// that node sends pass. An empty filter keeps everything, including the
/// extended frame.
#[test]
fn a_node_filtered_block_replays_that_nodes_traffic() {
    let path = write_log("roxy_can_block_node.asc");
    let dbc = load_dbc_str(
        "VERSION \"\"\n\nNS_ :\n\nBS_:\n\nBU_: ABS EngineECU\n\nBO_ 256 EngineMsg: 2 EngineECU\n SG_ S : 0|8@1+ (1,0) [0|0] \"\" EngineECU\n\nBO_ 512 AbsMsg: 1 ABS\n SG_ T : 0|8@1+ (1,0) [0|0] \"\" ABS\n",
    )
    .unwrap();

    let mut b = block(&path, Some("EngineECU"), Vec::new());
    b.load_queue(Some(&dbc), 0);
    assert_eq!(b.last_error, None);
    assert_eq!(
        b.queue.len(),
        3,
        "only the 0x100 frames are EngineECU's; the extended 0x200001 belongs to nobody"
    );

    let mut b = block(&path, Some("ABS"), Vec::new());
    b.load_queue(Some(&dbc), 0);
    assert_eq!(b.queue.len(), 1, "0x200 belongs to ABS");
    assert_eq!(b.queue[0].1.id, 0x200);

    let mut b = block(&path, None, Vec::new());
    b.load_queue(Some(&dbc), 0);
    assert_eq!(b.queue.len(), 5, "no filter keeps everything");
}

/// A load failure is a visible outcome: the block disables itself and
/// records why, instead of silently contributing nothing.
#[test]
fn a_failed_load_disables_the_block_and_names_the_reason() {
    let mut b = block("Z:/nowhere/never.asc", None, Vec::new());
    b.load_queue(None, 0);
    assert!(!b.enabled, "a broken block must not pretend to run");
    let err = b.last_error.clone().expect("the reason is recorded");
    assert!(!err.is_empty());
}

/// The cursor rewinds per run: the same queue replays from the top.
#[test]
fn a_rewound_block_replays_from_the_start() {
    let path = write_log("roxy_can_block_rw.asc");
    let mut b = block(&path, None, Vec::new());
    b.load_queue(None, 0);
    let mut out = Vec::new();
    b.poll(1_000_000, &mut out, 100);
    assert_eq!(out.len(), 5, "everything is due after the first second");
    b.rewind();
    let mut out = Vec::new();
    b.poll(0, &mut out, 100);
    assert_eq!(out.len(), 1, "the run starts over at zero");
    assert_eq!(out[0].data[0], 0x11);
}

/// A block loaded mid-run anchors at the current clock: the first frame
/// is due *now*, none are dated in the past, and the recorded spacing
/// runs forward from there. (Anchoring at zero would burst the whole
/// queue out immediately and then go silent forever.)
#[test]
fn a_mid_run_load_anchors_at_the_current_clock() {
    let path = write_log("roxy_can_block_anchor.asc");
    let mut b = block(&path, None, Vec::new());
    b.load_queue(None, 1_000_000); // the sim clock already reads 1 s

    let mut out = Vec::new();
    b.poll(1_000_000, &mut out, 100);
    assert_eq!(out.len(), 1, "only the first frame is due at the anchor");
    assert_eq!(out[0].t_us, 1_000_000, "stamped at the anchor");

    b.poll(1_005_000, &mut out, 100);
    assert_eq!(out.len(), 1, "recorded spacing still applies");

    b.poll(1_010_000, &mut out, 100);
    assert_eq!(
        out.len(),
        3,
        "two frames share rel 10 ms in the fixture; both are due now"
    );
    assert_eq!(out[1].t_us, 1_010_000);
    assert_eq!(out[1].id, 0x100);
    assert_eq!(out[2].t_us, 1_010_000);
    assert_eq!(out[2].id, 0x200);

    // Rewinding clears the anchor with the clock (reset_run context).
    b.rewind();
    let mut out = Vec::new();
    b.poll(0, &mut out, 100);
    assert_eq!(out.len(), 1, "back to the run's own zero");
    assert_eq!(out[0].t_us, 0);
}
