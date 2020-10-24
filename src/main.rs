extern crate ffmpeg_next as ffmpeg;
extern crate ffmpeg_sys_next as sys;

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
        .find(|&ref x| x.name() == "kmsgrab")
        .unwrap();

    // All the settings are listed here:
    // https://ffmpeg.org/ffmpeg-devices.html#x11grab
    let mut dict = Dictionary::new();
    dict.set("framerate", "60.01");
    dict.set("format", "bgr0");
    // optional
    // dict.set("video_size", "1920x1080");

    let hwctx_drm = unsafe {
        let mut ctx = sys::av_hwdevice_ctx_alloc(sys::AVHWDeviceType::AV_HWDEVICE_TYPE_DRM);

        let card_id = "/dev/dri/renderD128\0";

        let ret = sys::av_hwdevice_ctx_create(
            &mut ctx,
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_DRM,
            (card_id.as_ptr()) as *const _,
            Dictionary::new().as_mut_ptr(),
            0,
        );

        if ret < 0 {
            eprintln!(
                "Could not ctx_create: {:?}",
                ffmpeg::util::error::Error::from(ret)
            );
        }

        ctx
    };

    let hwctx_vaapi = unsafe {
        let mut ctx = sys::av_hwdevice_ctx_alloc(sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI);

        let card_id = "/dev/dri/renderD128\0";

        let ret = sys::av_hwdevice_ctx_create(
            &mut ctx,
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            (card_id.as_ptr()) as *const _,
            Dictionary::new().as_mut_ptr(),
            0,
        );

        if ret < 0 {
            eprintln!(
                "Could not ctx_create: {:?}",
                ffmpeg::util::error::Error::from(ret)
            );
        }

        ctx
    };

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

    /*
    unsafe {
        (*decoder.as_mut_ptr()).hw_frames_ctx = sys::av_buffer_ref(hwctx_drm);
    }
    */

    // set parameters (dunno which)
    decoder.set_parameters(input.parameters()).unwrap();

    eprintln!("Input format: {:?}", decoder.format());

    let buffer_params = format!(
        "video_size={}x{}:pix_fmt={}:time_base={}/{}:pixel_aspect={}/{}",
        decoder.width(),
        decoder.height(),
        decoder.format().descriptor().unwrap().name(),
        input.time_base().numerator(),
        input.time_base().denominator(),
        decoder.aspect_ratio().numerator(),
        decoder.aspect_ratio().denominator(),
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
        }

        // scaler and format converter
        // .parse("hwmap=derive_device=drm,hwmap=derive_device=vaapi,scale_vaapi=1920:1080:nv12")
        {}

        ffmpeg::util::log::set_level(ffmpeg::log::Level::Trace);

        // it appears this should be done in one statement
        filter
            .output("in", 0)
            .unwrap()
            .input("out", 0)
            .unwrap()
            // .parse("hwmap=derive_device=drm,hwmap=derive_device=vaapi,scale_vaapi=1920:1080:nv12")
            .parse("format=nv12")
            .unwrap();

        // set scaling threads
        /*
        {
            let mut _out = filter.get("Parsed_scale_0").unwrap();
        }
        */

        /*
        unsafe {
            let mut hwmap = filter.get("Parsed_hwdownload_0").unwrap();

            (*hwmap.as_mut_ptr()).hw_device_ctx = sys::av_buffer_ref(hwctx_drm);
        };
        */

        /*
        unsafe {
            let mut hwmap = filter.get("Parsed_hwupload_2").unwrap();
            // hwmap.set_pixel_format(ffmpeg::format::Pixel::VAAPI_VLD);

            (*hwmap.as_mut_ptr()).hw_device_ctx = sys::av_buffer_ref(hwctx_vaapi);
        };
        */

        filter.validate().unwrap();

        eprintln!("Created a filter:\n{}", filter.dump());

        filter
    };

    let encoding_codec = ffmpeg::encoder::find_by_name("h264").unwrap();

    let mut octx = ffmpeg::format::output_as(&"/dev/stdout", "mpegts").unwrap();

    let mut encoder = {
        let mut stream = octx.add_stream(encoding_codec).unwrap();

        stream.set_time_base(input.time_base());

        let codec = stream.codec();

        let mut encoder = codec.encoder().video().unwrap();

        unsafe {
            eprintln!("hwctx_vaapi: {:?}", *hwctx_vaapi);

            (*encoder.as_mut_ptr()).hw_frames_ctx = sys::av_buffer_ref(hwctx_vaapi);
        }

        let codec_opts = {
            let mut dict = Dictionary::new();

            dict.set("preset", "ultrafast");
            dict.set("tune", "zerolatency");

            dict
        };

        eprintln!("input time base: {:?}", input.time_base());

        encoder.set_time_base(input.time_base());
        encoder.set_format(ffmpeg::format::Pixel::VAAPI_VLD);
        encoder.set_width(decoder.width());
        encoder.set_height(decoder.height());
        encoder.set_frame_rate(decoder.frame_rate());
        encoder.set_bit_rate(5000);

        encoder.open_as_with(encoding_codec, codec_opts).unwrap()
    };

    // encoder.set_bit_rate(5);

    octx.write_header().unwrap();

    let video_stream_index = input.index();

    let in_time_base = input.time_base();
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

        let mut decoded = frame::Video::empty();

        {
            // decode step
            let decode_start = Instant::now();

            decoder.send_packet(&packet).unwrap();
            decoder.receive_frame(&mut decoded).unwrap();

            decode_counter += decode_start.elapsed();

            let mut filtered = frame::Video::empty();

            // filter feed step
            let filter_start = Instant::now();
            {
                filter.get("in").unwrap().source().add(&decoded).unwrap();
                // sink filter
                while filter
                    .get("out")
                    .unwrap()
                    .sink()
                    .frame(&mut filtered)
                    .is_ok()
                {
                    eprintln!("filtered");
                }
            }
            filter_counter += filter_start.elapsed();

            // encode
            let encode_start = Instant::now();

            {
                let mut to_stream = ffmpeg::Packet::empty();

                encoder.send_frame(&filtered).unwrap();

                while encoder.receive_packet(&mut to_stream).is_ok() {
                    // prepare packet for sending to octx
                    to_stream.set_stream(0);
                    to_stream.rescale_ts(in_time_base, out_time_base);

                    // write packet to octx;
                    to_stream.write_interleaved(&mut octx).unwrap();
                }
            }

            encode_counter += encode_start.elapsed();

            // fps stuff
            frames += 1;

            if frames == 60 * 5 {
                eprintln!("Setting bitrate");
                encoder.set_bit_rate(100000);
            }

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
