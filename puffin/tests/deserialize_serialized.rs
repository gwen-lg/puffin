mod common;

use std::{
    env,
    io::{Read, Seek},
    thread,
    time::Duration,
};

use memfile::MemFile;

fn run_write(file: MemFile) {
    //let frame_data = BufWriter::new(File::create(FILE_NAME).unwrap());
    let _frame_writer = common::init_frames_writer(file);

    //println!("set_scopes_on(true)");
    puffin::set_scopes_on(true);
    // need this to enable capture

    common::example_run();

    //println!("set_scopes_on(false)");
    puffin::set_scopes_on(false);
    puffin::GlobalProfiler::lock().new_frame();
    //Force to get last frame
}

fn run_read(mut file: MemFile) {
    //    let mut frame_reader = BufReader::new(File::open(FILE_NAME).unwrap());
    file.rewind().unwrap();

    let output_content = env::var("RUST_BACKTRACE")
        .map(|value| value == "full" || value == "1" || value == "debug")
        .unwrap_or(false);

    if output_content {
        let contents = file.bytes().collect::<Result<Vec<_>, _>>().unwrap();
        std::fs::write("tests/deserialize_serialized.puffin", contents).unwrap();
    } else {
        let _ = puffin::FrameView::read(&mut file).expect("read :");
    }
}

#[test]
fn deserialize_serialized() {
    let file = MemFile::create_default("deserialize_serialized.puffin").unwrap();
    run_write(file.try_clone().unwrap());
    thread::sleep(Duration::from_secs(1));
    run_read(file);
}
