use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread::sleep,
    time::Duration,
};

use puffin::{FrameData, GlobalProfiler};

fn process_1() {
    puffin::profile_function!();
    sub_process_1_1();
    (0..2).for_each(|_| sub_process_1_2());
}

fn sub_process_1_1() {
    puffin::profile_function!();
    sleep(Duration::from_millis(1));
}

fn sub_process_1_2() {
    puffin::profile_function!();
    sleep(Duration::from_micros(2));
}

#[test]
fn single_frame() {
    fn profiler_sink(frame_data: Arc<FrameData>) {
        let frame_meta = frame_data.meta();
        assert_eq!(frame_meta.frame_index, 0);
        assert_eq!(frame_meta.num_scopes, 4);
    }

    // Init profiler sink and enable capture
    let sink_id = GlobalProfiler::lock().add_sink(Box::new(profiler_sink));
    puffin::set_scopes_on(true);

    // Run process
    process_1();

    // End frame, and uninit profiler
    puffin::GlobalProfiler::lock().new_frame();
    GlobalProfiler::lock().remove_sink(sink_id);
}

#[test]
fn multiple_frame() {
    const NB_LOOP: usize = 10;
    fn profiler_sink(frame_data: Arc<FrameData>, frame_count: Arc<AtomicUsize>) {
        let previous_count = frame_count.fetch_add(1, Ordering::Relaxed);
        let frame_meta = frame_data.meta();
        assert_eq!(frame_meta.frame_index, previous_count as u64);
        assert_eq!(frame_meta.num_scopes, 4);
    }

    // Init profiler sink and enable capture
    let frame_count = Arc::new(AtomicUsize::default());
    let frame_count_clone = frame_count.clone();
    let sink_id = GlobalProfiler::lock().add_sink(Box::new(move |frame_data| {
        profiler_sink(frame_data, frame_count_clone.clone());
    }));
    puffin::set_scopes_on(true);

    // Run process
    std::iter::repeat_n((), NB_LOOP).for_each(|_| {
        process_1();
        puffin::GlobalProfiler::lock().new_frame();
    });

    let frame_count = frame_count.load(Ordering::Relaxed);
    assert_eq!(frame_count, NB_LOOP);

    // End frame, and uninit profiler
    GlobalProfiler::lock().remove_sink(sink_id);
}
