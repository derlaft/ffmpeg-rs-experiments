extern crate ffmpeg_next as ffmpeg;
extern crate ffmpeg_sys_next as sys;

use ffmpeg::filter;
use ffmpeg::frame;
use ffmpeg::media::Type;
use ffmpeg::Dictionary;
use ffmpeg::Error;

use std::ffi::CString;
use std::time::{Duration, Instant};

macro_rules! check {
    ($expr:expr) => {
        match $expr {
            0 => Ok(()),
            e => Err(Error::from(e)),
        }
    };
}

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

    eprintln!("Input format: {}", decoder.format() as u32);

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
    let mut filter = {
        let mut filter = filter::Graph::new();

        filter
            .add(
                &filter::find("buffer").unwrap(),
                // name
                "in",
                // params
                &buffer_params,
            )
            .unwrap();

        filter
            .add(
                &filter::find("buffersink").unwrap(),
                // name
                "out",
                // params empty
                "",
            )
            .unwrap();

        // set in pixel format
        {
            let mut inp = filter.get("in").unwrap();
            inp.set_pixel_format(decoder.format());

            /*
            unsafe {
                (*inp.as_mut_ptr()).nb_threads = 4;
            }
            */
        }

        // set out pixel format
        {
            let mut out = filter.get("out").unwrap();
            out.set_pixel_format(ffmpeg::format::Pixel::NV12);
            // out.set_pixel_format(ffmpeg::format::Pixel::BGRZ);

            /*
            unsafe {
                (*out.as_mut_ptr()).nb_threads = 4;
            }
            */
        }

        // scaler and format converter
        {}

        // it appears this should be done in one statement
        filter
            .output("in", 0)
            .unwrap()
            .input("out", 0)
            .unwrap()
            .parse("scale")
            .unwrap();

        // set scaling threads
        {
            let mut _out = filter.get("Parsed_scale_0").unwrap();
        }

        filter.validate().unwrap();

        eprintln!("Created a filter:\n{}", filter.dump());

        filter
    };

    let encoding_codec = ffmpeg::encoder::find(ffmpeg::codec::Id::H264).unwrap();

    let mut octx = ffmpeg::format::output_as(&"/dev/stdout", "mpegts").unwrap();

    let mut encoder = {
        let mut stream = octx.add_stream(encoding_codec).unwrap();

        // stream.set_time_base(decoder.time_base() );

        stream.set_time_base(decoder.time_base());

        let codec = stream.codec();

        let mut encoder = codec.encoder().video().unwrap();

        /*
        unsafe {
            let pd = (*encoder.as_mut_ptr()).priv_data;

            let name = CString::new("preset").unwrap();
            let value = CString::new("ultrafast").unwrap();

            check!(sys::av_opt_set(
                pd,
                name.as_ptr(),
                value.as_ptr(),
                sys::AV_OPT_SEARCH_CHILDREN
            ))
        }
        .unwrap();
        */

        unsafe {
            let pd = (*encoder.as_mut_ptr()).priv_data;

            let name = CString::new("tune").unwrap();
            let value = CString::new("zerolatency").unwrap();

            check!(sys::av_opt_set(
                pd,
                name.as_ptr(),
                value.as_ptr(),
                sys::AV_OPT_SEARCH_CHILDREN
            ))
        }
        .unwrap();

        unsafe {
            let pd = (*encoder.as_mut_ptr()).priv_data;

            let name = CString::new("preset").unwrap();
            let value = CString::new("ultrafast").unwrap();

            check!(sys::av_opt_set(
                pd,
                name.as_ptr(),
                value.as_ptr(),
                sys::AV_OPT_SEARCH_CHILDREN
            ))
        }
        .unwrap();

        encoder.set_time_base(decoder.time_base());
        encoder.set_format(ffmpeg::format::Pixel::NV12);
        encoder.set_width(decoder.width());
        encoder.set_height(decoder.height());
        encoder.set_frame_rate(decoder.frame_rate());
        // encoder.set_bit_rate(1000);

        encoder.open_as(encoding_codec).unwrap()
    };

    octx.write_header().unwrap();

    let video_stream_index = input.index();

    let in_time_base = decoder.time_base();
    let out_time_base = octx.stream(0).unwrap().time_base();

    // some timing counter shit
    let mut frames = 0;
    let mut capture_start = Instant::now();
    let mut decode_counter = Duration::new(0, 0);
    let mut filter_counter = Duration::new(0, 0);
    let mut encode_counter = Duration::new(0, 0);

    for (stream, packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue;
        }

        eprintln!("Incoming pts: {:?}", packet.pts());

        let mut decoded = frame::Video::empty();

        {
            // decode step
            let decode_start = Instant::now();
            decoder.decode(&packet, &mut decoded).unwrap();

            decode_counter += decode_start.elapsed();

            let mut filtered = frame::Video::empty();

            // filter feed step
            let filter_start = Instant::now();
            {
                filter.get("in").unwrap().source().add(&decoded).unwrap();
                // sink filter
                while let Ok(..) = filter.get("out").unwrap().sink().frame(&mut filtered) {
                    eprintln!("filtered");
                }
            }
            filter_counter += filter_start.elapsed();

            eprintln!("filtered pts: {:?}", filtered.pts());

            // encode
            let encode_start = Instant::now();
            {
                let mut to_stream = ffmpeg::Packet::empty();

                encoder.encode(&filtered, &mut to_stream).unwrap();

                eprintln!("Encoded packet: {:?}", to_stream.size());
                to_stream.set_stream(0);
                // to_stream.set_pts(packet.pts());
                //
                //
                eprintln!("Encoded pts: {:?}", to_stream.pts());

                to_stream.set_pts(packet.pts());
                // to_stream.rescale_ts(in_time_base, out_time_base);
                eprintln!("set pts ok");
                to_stream.write_interleaved(&mut octx).unwrap();
                eprintln!("Wrote packet");
            }

            encode_counter += encode_start.elapsed();

            // fps stuff
            frames += 1;

            if frames % 60 == 0 {
                eprintln!(
                    "Frames: {}\t\tDecode: {:?}\t\tFilter: {:?}\t\tEncode: {:?}\n",
                    60.0 / capture_start.elapsed().as_secs_f32(),
                    decode_counter.div_f32(60.0),
                    filter_counter.div_f32(60.0),
                    encode_counter.div_f32(60.0),
                );

                capture_start = Instant::now();
                decode_counter = Duration::new(0, 0);
                filter_counter = Duration::new(0, 0);
                encode_counter = Duration::new(0, 0);
            }
        }
    }

    eprintln!("Context opened!")
}
