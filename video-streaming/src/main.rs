// #[macro_use]

use anyhow::Result;
use base64::encode;
use bus::{Bus, BusReader};
// use futures_util::{SinkExt, StreamExt};
use image::codecs;
use image::ColorType;
use image::ImageBuffer;
use image::Rgb;
use nokhwa::{Camera, CameraFormat, CaptureAPIBackend, FrameFormat};
use rav1e::prelude::ChromaSampling;
use rav1e::*;
use rav1e::{config::SpeedSettings, prelude::FrameType};
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::time::sleep;
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, thread};
use tide::log::{debug, error, info, warn};
use tide::Request;
use tide_websockets::{Message, WebSocket, WebSocketConnection};
use tokio::sync::watch;
use std::io::BufWriter;
// use portaudio as pa;



#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
struct VideoPacket {
    data: Option<String>,
    frame_type: Option<String>,
    epoch_time: Duration,
    encoding: Encoder,
}

#[derive(Serialize, Deserialize, Debug)]
struct SoundPacket {
    pub data: Vec<u8>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
enum Encoder {
    MJPEG,
    AV1,
}

impl FromStr for Encoder {
    type Err = ();

    fn from_str(input: &str) -> Result<Encoder, Self::Err> {
        match input {
            "MJPEG" => Ok(Encoder::MJPEG),
            "AV1" => Ok(Encoder::AV1),
            _ => Err(()),
        }
    }
}


static THRESHOLD_MILLIS: u128 = 1000;


// const SAMPLE_RATE: f64 = 44_100.0;
// const FRAMES: u32 = 256;
// const CHANNELS: i32 = 2;
// const INTERLEAVED: bool = true;

// fn run() -> Result<(), pa::Error> {
//     let pa = pa::PortAudio::new()?;

//     println!("PortAudio:");
//     println!("version: {}", pa.version());
//     println!("version text: {:?}", pa.version_text());
//     println!("host count: {}", pa.host_api_count()?);

//     let default_host = pa.default_host_api()?;
//     println!("default host: {:#?}", pa.host_api_info(default_host));

//     let def_input = pa.default_input_device()?;
//     let input_info = pa.device_info(def_input)?;
//     println!("Default input device info: {:#?}", &input_info);

//     // Construct the input stream parameters.
//     let latency = input_info.default_low_input_latency;
//     let input_params = pa::StreamParameters::<f32>::new(def_input, CHANNELS, INTERLEAVED, latency);

//     // let def_output = pa.default_output_device()?;
//     // let output_info = pa.device_info(def_output)?;
//     println!("Default output device info: {:#?}", &output_info);

//     // Construct the output stream parameters.
//     let latency = output_info.default_low_output_latency;
//     // let output_params = pa::StreamParameters::new(def_output, CHANNELS, INTERLEAVED, latency);

//     // Check that the stream format is supported.
//     pa.is_duplex_format_supported(input_params, output_params, SAMPLE_RATE)?;

//     // Construct the settings with which we'll open our duplex stream.
//     let settings = pa::InputSettings::new(input_params, SAMPLE_RATE, FRAMES);

//     // Once the countdown reaches 0 we'll close the stream.
//     let mut count_down = 3.0;

//     // Keep track of the last `current_time` so we can calculate the delta time.
//     let mut maybe_last_time = None;

//     // We'll use this channel to send the count_down to the main thread for fun.
//     let (sender, receiver) = std::sync::mpsc::channel();

//     // A callback to pass to the non-blocking stream.
//     let callback = move |pa::InputCallbackArgs {
//                              in_buffer,
//                              out_buffer,
//                              frames,
//                              time,
//                              ..
//                          }| {
//         let current_time = time.current;
//         let prev_time = maybe_last_time.unwrap_or(current_time);
//         let dt = current_time - prev_time;
//         count_down -= dt;
//         maybe_last_time = Some(current_time);

//         assert!(frames == FRAMES as usize);
//         sender.send(count_down).ok();

//         // Pass the input straight to the output - BEWARE OF FEEDBACK!
//         for (output_sample, input_sample) in out_buffer.iter_mut().zip(in_buffer.iter()) {
//             *output_sample = *input_sample;
//         }

//         if count_down > 0.0 {
//             pa::Continue
//         } else {
//             pa::Complete
//         }
//     };

//     // Construct a stream with input and output sample types of f32.
//     let mut stream = pa.open_non_blocking_stream(settings, callback)?;

//     stream.start()?;

//     // Loop while the non-blocking stream is active.
//     while let true = stream.is_active()? {
//         // Do some stuff!
//         while let Ok(count_down) = receiver.try_recv() {
//             println!("count_down: {:?}", count_down);
//         }
//     }

//     stream.stop()?;

//     Ok(())
// }

#[tokio::main]
async fn main() -> Result<()> {
    let (send_cam, recv_cam) = watch::channel(String::new());

    let state = TideState {
        counter: Arc::new(Mutex::new(16)),
    };
    let client_counter = state.counter.clone();

    env_logger::init();
    let mut enc = EncoderConfig::default();
    let width = 640;
    let height = 480;
    let video_device_index: usize = env::var("VIDEO_DEVICE_INDEX")
        .ok()
        .map(|n| n.parse::<usize>().ok())
        .flatten()
        .unwrap_or(0);
    let framerate: u32 = env::var("FRAMERATE")
        .ok()
        .map(|n| n.parse::<u32>().ok())
        .flatten()
        .unwrap_or(10u32);
    let encoder = env::var("ENCODER")
        .ok()
        .map(|o| Encoder::from_str(o.as_ref()).ok())
        .flatten()
        .unwrap_or(Encoder::AV1);

    warn!("Framerate {framerate}");
    enc.width = width;
    enc.height = height;
    enc.bit_depth = 8;
    enc.error_resilient = true;
    enc.speed_settings = SpeedSettings::from_preset(10);
    enc.rdo_lookahead_frames = 1;
    enc.min_key_frame_interval = 20;
    enc.max_key_frame_interval = 50;
    enc.low_latency = true;
    enc.min_quantizer = 50;
    enc.quantizer = 100;
    enc.still_picture = false;
    enc.tiles = 4;
    enc.chroma_sampling = ChromaSampling::Cs444;

    let (audio_send, audio_recv) = watch::channel(String::new());


    // Add counter to warp so that we can access it when we add/remove connections

    let cfg = Config::new().with_encoder_config(enc).with_threads(4);

    let (fps_tx, fps_rx): (Sender<u128>, Receiver<u128>) = mpsc::channel();
    let (cam_tx, cam_rx): (
        Sender<(ImageBuffer<Rgb<u8>, Vec<u8>>, u128)>,
        Receiver<(ImageBuffer<Rgb<u8>, Vec<u8>>, u128)>,
    ) = mpsc::channel();

    let devices = nokhwa::query_devices(CaptureAPIBackend::GStreamer)?;
    info!("available cameras: {:?}", devices);

    let fps_thread = tokio::spawn(async move {
        let mut num_frames = 0;
        let mut now_plus_1 = since_the_epoch().as_millis() + 1000;
        warn!("Starting fps loop");
        loop {
            match fps_rx.recv() {
                Ok(dur) => {
                    if now_plus_1 < dur {
                        warn!("FPS: {:?}", num_frames);
                        num_frames = 0;
                        now_plus_1 = since_the_epoch().as_millis() + 1000;
                    } else {
                        num_frames += 1;
                    }
                }
                Err(e) => {
                    error!("Receive error: {:?}", e);
                }
            }
        }
    });

    let camera_thread = tokio::spawn(async move {
        loop {
            {
                info!("waiting for browser...");
                sleep(Duration::from_millis(200)).await;
                let counter = client_counter.lock().unwrap();
                if *counter <= 0 {
                    continue;
                }
            }
            let mut camera = Camera::with_backend(
                video_device_index, // index
                Some(CameraFormat::new_from(
                    width as u32,
                    height as u32,
                    FrameFormat::MJPEG,
                    framerate,
                )),
                CaptureAPIBackend::GStreamer // format
            )
            .unwrap();
            camera.open_stream().unwrap();
            loop {
                {
                    let counter = client_counter.lock().unwrap();
                    if *counter <= 0 {
                        break;
                    }
                }
                let frame = camera.frame().unwrap();
                cam_tx.send((frame, since_the_epoch().as_millis())).unwrap();
            }
        }
    });

    let encoder_thread = tokio::spawn(async move {
        loop {
            let fps_tx_copy = fps_tx.clone();
            let mut ctx: Context<u8> = cfg.new_context().unwrap();
            loop {
                let (mut frame, age) = cam_rx.recv().unwrap();
                // If age older than threshold, throw it away.
                let frame_age = since_the_epoch().as_millis() - age;
                debug!("frame age {}", frame_age);
                if frame_age > THRESHOLD_MILLIS {
                    debug!("throwing away old frame with age {} ms", frame_age);
                    continue;
                }
                if encoder == Encoder::MJPEG {
                    let mut buf: Vec<u8> = Vec::new();
                    let mut jpeg_encoder =
                        codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80);
                    jpeg_encoder
                        .encode_image(&frame)
                        .map_err(|e| error!("{:?}", e))
                        .unwrap();
                    let frame = VideoPacket {
                        data: Some(encode(&buf)),
                        frame_type: None,
                        epoch_time: since_the_epoch(),
                        encoding: encoder.clone(),
                    };
                    let json = serde_json::to_string(&frame).unwrap();
                    send_cam.send(json);
                    fps_tx_copy.send(since_the_epoch().as_millis()).unwrap();
                    continue;
                }
                let mut r_slice: Vec<u8> = vec![];
                let mut g_slice: Vec<u8> = vec![];
                let mut b_slice: Vec<u8> = vec![];
                for pixel in frame.pixels_mut() {
                    let (r, g, b) = to_ycbcr(pixel);
                    r_slice.push(r);
                    g_slice.push(g);
                    b_slice.push(b);
                }
                let planes = vec![r_slice, g_slice, b_slice];
                debug!("Creating new frame");
                let mut frame = ctx.new_frame();
                let encoding_time = Instant::now();
                for (dst, src) in frame.planes.iter_mut().zip(planes) {
                    dst.copy_from_raw_u8(&src, enc.width, 1);
                }

                match ctx.send_frame(frame) {
                    Ok(_) => {
                        debug!("queued frame");
                    }
                    Err(e) => match e {
                        EncoderStatus::EnoughData => {
                            debug!("Unable to append frame to the internal queue");
                        }
                        _ => {
                            panic!("Unable to send frame");
                        }
                    },
                }
                debug!("receiving encoded frame");
                match ctx.receive_packet() {
                    Ok(pkt) => {
                        debug!("time encoding {:?}", encoding_time.elapsed());
                        debug!("read thread: base64 Encoding packet {}", pkt.input_frameno);
                        let frame_type = if pkt.frame_type == FrameType::KEY {
                            "key"
                        } else {
                            "delta"
                        };
                        let time_serializing = Instant::now();
                        let data = encode(pkt.data);
                        debug!("read thread: base64 Encoded packet {}", pkt.input_frameno);
                        let frame = VideoPacket {
                            data: Some(data),
                            frame_type: Some(frame_type.to_string()),
                            epoch_time: since_the_epoch(),
                            encoding: encoder.clone(),
                        };
                        let json = serde_json::to_string(&frame).unwrap();
                        send_cam.send(json);
                        debug!("time serializing {:?}", time_serializing.elapsed());
                        fps_tx_copy.send(since_the_epoch().as_millis()).unwrap();
                    }
                    Err(e) => match e {
                        EncoderStatus::LimitReached => {
                            warn!("read thread: Limit reached");
                        }
                        EncoderStatus::Encoded => debug!("read thread: Encoded"),
                        EncoderStatus::NeedMoreData => debug!("read thread: Need more data"),
                        _ => {
                            warn!("read thread: Unable to receive packet");
                        }
                    },
                }
            }
        }
    });

    let mut app = tide::with_state(Arc::new(state));
    let (playback_sender, playback_recv) = std::sync::mpsc::channel();

    let play_back_thread = std::thread::spawn(move || {
        //  create the rodio output stream
        loop {
           if let Ok(packet) =  playback_recv.recv() {
            // convert packet.data to wav;
            // decode and play on output stream
           }
        }
    });

    app.at("/ws").get(WebSocket::new(
        move |req: Request<std::sync::Arc<TideState>>, wsc: WebSocketConnection| {
            let rx = WatchStream::new(recv_cam.clone());

            async move {
                println!("Web socketss {:?}", wsc);
                let mut wsc_clone = wsc.clone();

                tokio::spawn(async move {
                    while let Some(Ok(Message::Text(msg))) = tokio_stream::StreamExt::next(&mut wsc_clone).await {
                        if let Ok(packet) = serde_json::from_str::<SoundPacket>(&msg){
                            // send sound packet to playback thread
                            playback_sender.send(packet).unwrap();
                        }
                    }
                });

                let _ = client_connection(wsc, req, rx).await?;
                Ok(())
            }
        },
    ));

    app.listen("0.0.0.0:8080").await?;
    encoder_thread.await.unwrap();
    fps_thread.await.unwrap();
    camera_thread.await.unwrap();
    play_back_thread.join();
    Ok(())
}

fn clamp(val: f32) -> u8 {
    return (val.round() as u8).max(0_u8).min(255_u8);
}

fn to_ycbcr(pixel: &Rgb<u8>) -> (u8, u8, u8) {
    let [r, g, b] = pixel.0;

    let y = 16_f32 + (65.481 * r as f32 + 128.553 * g as f32 + 24.966 * b as f32) / 255_f32;
    let cb = 128_f32 + (-37.797 * r as f32 - 74.203 * g as f32 + 112.000 * b as f32) / 255_f32;
    let cr = 128_f32 + (112.000 * r as f32 - 93.786 * g as f32 - 18.214 * b as f32) / 255_f32;

    return (clamp(y), clamp(cb), clamp(cr));
}

pub struct TideState {
    counter: Arc<Mutex<u16>>,
}

use tokio_stream::wrappers::WatchStream;


pub async fn client_connection(
    wsc: WebSocketConnection,
    request: Request<Arc<TideState>>,
    mut recv: tokio_stream::wrappers::WatchStream<String>,
) -> tide::Result {
    let counter = &request.state().counter;
    info!("establishing client connection... {:?}", wsc);

    {
        info!("blocking before adding connection {:?}", counter);
        let mut counter_ref = counter.lock().unwrap();
        *counter_ref = *counter_ref + 1;
        info!("adding connection, connection counter: {:?}", *counter_ref);
        drop(counter_ref);
    }
    loop {
        let next = tokio_stream::StreamExt::next(&mut recv).await.unwrap();
        debug!("Forwarding video message");
        let time_serializing = Instant::now();
        match wsc.send(Message::text(next)).await {
            Ok(_) => {}
            Err(_e) => {
                info!("blocking before removing connection {:?}", counter);
                let mut counter_ref = counter.lock().unwrap();
                *counter_ref = *counter_ref - 1;
                info!(
                    "Removing connection, connection counter: {:?}",
                    *counter_ref
                );
                break;
            }
        }
        debug!("web_socket serializing {:?}", time_serializing.elapsed());
    }

    Ok(format!("Action executed").into())
}

pub fn since_the_epoch() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
}
