use color_eyre::eyre::eyre;
use color_eyre::eyre::Result;
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use dasp::interpolate::linear::Linear;
use dasp::{signal, Signal};
use std::io::Cursor;
use std::sync::mpsc::Sender;
use std::sync::{mpsc::channel, Arc, Mutex};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

type ClipHandle = Arc<Mutex<Option<AudioClip>>>;
type StateHandle = Arc<Mutex<Option<(usize, Vec<u16>, Sender<()>)>>>;

#[derive(Clone, Debug)]
pub struct AudioClip {
    pub samples: Vec<u16>,
    pub sample_rate: u32,
}

impl AudioClip {
    pub fn record(sender: tokio::sync::watch::Sender<Vec<u16>>) -> Result<AudioClip> {
        //get the host
        let host = cpal::default_host();

        //get the default input device
        let device = host
            .default_input_device()
            .ok_or_else(|| eyre!("No input device!"))?;
        println!("Input device: {}", device.name()?);

        //get default config - channels, sample_rate,buffer_size, sample_format
        let config = device.default_input_config()?;

        //init a audio clip
        let clip = AudioClip {
            samples: Vec::new(),
            sample_rate: config.sample_rate().0,
        };

        let clip = Arc::new(Mutex::new(Some(clip)));

        // Run the input stream on a separate thread.
        let clip_2 = clip.clone();

        println!("Begin recording...");

        let err_fn = move |err| {
            eprintln!("an error occurred on stream: {}", err);
        };
        //get number of channels
        let channels = config.channels();

        //create stream
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data, _: &_| write_input_data::<f32>(data, channels, &sender),
                err_fn,
            )?,
            _ => panic!("Unsupported"),

        };

        //run stream
        stream.play()?;

        loop {

        }
    }

    pub fn import(buffer: Vec<u8>) -> Result<AudioClip> {
        let mut samples = vec![];
        for i in buffer.chunks(2) {
            if i.len() != 2 {
                println!("Incomplete buffer");
                break;
            }
            let lower_bit = (i[0] as u16) << 8;
            let higher_bit = i[1] as u16;
            let sample = lower_bit | higher_bit;
            samples.push(sample)
        }

        Ok(AudioClip {
            samples,
            sample_rate: 44100,
        })
    }

    pub fn play(&self) -> Result<()> {
        //get the host
        let host = cpal::default_host();

        //get the default output device
        let device = host
            .default_output_device()
            .ok_or_else(|| eyre!("No output device!"))?;
        println!("Output device: {}", device.name()?);

        //get default config - channels, sample_rate,buffer_size, sample_format
        let config = device.default_output_config()?;

        println!("Begin playback...");

        //get number of channels
        let channels = config.channels();

        let err_fn = move |err| {
            eprintln!("an error occurred on stream: {}", err);
        };
        println!("Playback1");

        let (done_tx, done_rx) = channel::<()>();

        println!("Playback2");

        let state = (0, self.samples.clone(), done_tx);
        println!("Playback3");

        let state = Arc::new(Mutex::new(Some(state)));
        println!("Playback4");

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device
                .build_output_stream(
                    &config.into(),
                    move |data, _: &_| write_output_data::<f32>(data, channels, &state),
                    err_fn,
                )
                .map_err(|_e| {
                    println!("U16 error? {}", _e);
                    tide::http::Error::from_str(tide::StatusCode::BadRequest, "Error happened")
                })
                .unwrap(),
            _ => panic!("Unsupported"),
        };

        println!("Playback5");

        stream
            .play()
            .map_err(|_e| {
                println!("Play error? {}", _e);
                tide::http::Error::from_str(tide::StatusCode::BadRequest, "Error happened")
            })
            .unwrap();

        println!("Playback6");

        done_rx.recv()?;

        println!("Playback7");

        Ok(())
    }
}

pub fn write_input_data<T>(
    input: &[T],
    channels: u16,
    sender: &tokio::sync::watch::Sender<Vec<u16>>,
) where
    T: cpal::Sample,
{
    let mut samples = vec![];
    for frame in input.chunks(channels.into()) {
        samples.push(frame[0].to_u16());
    }
    // send samples to the thread that sends it to client
    sender.send(samples).unwrap();
}

pub fn write_output_data<T>(output: &mut [T], channels: u16, writer: &StateHandle)
where
    T: cpal::Sample,
{
    if let Ok(mut guard) = writer.try_lock() {
        if let Some((i, clip_samples, done)) = guard.as_mut() {
            for frame in output.chunks_mut(channels.into()) {
                for sample in frame.iter_mut() {
                    *sample = cpal::Sample::from(clip_samples.get(*i).unwrap_or(&0u16));
                }
                *i += 1;
            }

            if *i >= clip_samples.len() {
                if let Err(_) = done.send(()) {
                    //playback has already stopped
                }
            }
        }
    }
}
