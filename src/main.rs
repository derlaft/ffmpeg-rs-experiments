extern crate ffmpeg_next as ffmpeg;

use ffmpeg::filter;
use ffmpeg::frame;
use ffmpeg::media::Type;
use ffmpeg::Dictionary;
use std::time::{Duration, Instant};

fn main() {
    ffmpeg::init().unwrap();

    // find x11grab device
    let x11grab = ffmpeg::format::list()
        .into_iter()
        .find(|&ref x| x.name() == "x11grab")
        .unwrap();

    // All the settings are listed here:
    // https://ffmpeg.org/ffmpeg-devices.html#x11grab
    let mut dict = Dictionary::new();
    dict.set("framerate", "60.01");
    dict.set("draw_mouse", "0");
    // optional
    // dict.set("video_size", "1920x1080");

    // TODO: configure it?
    let display = ":0".to_string();

    // open input demuxer
    let mut ictx = ffmpeg::format::open_with(&display, &x11grab, dict)
        .unwrap()
        .input();

    // dunno what that means just yet, copied from an example
    let input = ictx
        .streams()
        .best(Type::Video)
        .ok_or_else(|| ffmpeg::Error::StreamNotFound)
        .unwrap();

    // create video decoder
    let mut decoder = input.codec().decoder().video().unwrap();
    // set parameters (dunno which)
    decoder.set_parameters(input.parameters()).unwrap();

    println!("Input format: {}", decoder.format() as u32);

    let buffer_params = format!(
        "video_size={}x{}:pix_fmt={}:time_base={}:pixel_aspect={}",
        decoder.width(),
        decoder.height(),
        decoder.format().descriptor().unwrap().name(),
        decoder.time_base(),
        decoder.aspect_ratio(),
    );

    // create a filter
    // original ffmpeg filter:
    let mut filter = filter::Graph::new();

    filter
        .add(
            &filter::find("buffer").unwrap(),
            // type
            "in",
            // params
            &buffer_params,
        )
        .unwrap();

    filter
        .add(
            &filter::find("buffersink").unwrap(),
            // type
            "out",
            // params empty
            "",
        )
        .unwrap();

    // set in pixel format
    {
        let mut inp = filter.get("in").unwrap();
        inp.set_pixel_format(decoder.format());
    }

    // set out pixel format
    {
        let mut out = filter.get("out").unwrap();
        out.set_pixel_format(ffmpeg::format::Pixel::NV12);
        // out.set_pixel_format(ffmpeg::format::Pixel::BGRZ);
    }

    // it appears this should be done in one statement
    filter
        .output("in", 0)
        .unwrap()
        .input("out", 0)
        .unwrap()
        .parse("copy")
        .unwrap();

    filter.validate().unwrap();

    println!("Created a filter:\n{}", filter.dump());

    let video_stream_index = input.index();

    let mut frames = 0;
    let mut capture_start = Instant::now();
    let mut decoded = frame::Video::empty();

    let mut decode_counter = Duration::new(0, 0);
    let mut filter_counter = Duration::new(0, 0);

    for (stream, packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue;
        }

        // decode step
        {
            let decode_start = Instant::now();
            decoder.decode(&packet, &mut decoded).unwrap();
            decode_counter += decode_start.elapsed();

            // filter feed step
            let filter_start = Instant::now();
            filter.get("in").unwrap().source().add(&decoded).unwrap();

            // sink filter
            let mut a = 0;
            while let Ok(..) = filter.get("out").unwrap().sink().frame(&mut decoded) {
                a += 1;
            }
            if a >= 2 {
                panic!("Too many sinks: {}");
            }

            filter_counter += filter_start.elapsed();

            frames += 1;

            let fps = 60.0 / capture_start.elapsed().as_secs_f32();

            if frames % 60 == 0 {
                let lstart = Instant::now();

                println!(
                    "Frames: {}\t\tDecode: {:?}\t\tFilter: {:?}\n",
                    fps,
                    decode_counter.div_f32(60.0),
                    filter_counter.div_f32(60.0),
                );
                capture_start = Instant::now();
                decode_counter = Duration::new(0, 0);
                filter_counter = Duration::new(0, 0);

                println!("Eh: {:?}", lstart.elapsed());
            }
        }
    }

    println!("Context opened!")
}
