use crate::parser::{parse_raw_frame, read_index, VideoCaptureFormat};
use chrono::Local;
use mp4::{MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use zerocopy::AsBytes;

/// Function that converts a .vraw file to an .mp4 file.
/// NOTE: Currently only HEVC is supported!!!
///
/// input: path to .vraw file
///
/// output: name of the gengerated .mp4 file. If None is specified the file will
/// be named after the input and the time of generation.
pub fn convert_vraw_to_mp4(input: &String, output: Option<String>) -> Result<(), String> {
    let input_file = File::open(input).map_err(|_| "vraw_convert: failed to open file")?;

    let output = output.unwrap_or_else(|| {
        let input_path = Path::new(&input);

        let output_file_name = input_path.file_name().unwrap().to_str().unwrap();

        let output_file_name = format!(
            "{}_{}.mp4",
            output_file_name.trim_end_matches(".vraw"),
            Local::now().format("%Y-%m-%dT%H_%M_%S")
        );

        input_path
            .ancestors()
            .nth(2)
            .unwrap()
            .join(output_file_name)
            .to_string_lossy()
            .to_string()
    });

    let mut f: BufReader<File> = BufReader::new(input_file);

    let entries =
        read_index(&mut f).map_err(|e| format!("vraw_convert: failed to read index: {e}"))?;

    if entries.is_empty() {
        return Err("vraw_convert: index contains no frames".into());
    }

    let config: Mp4Config = Mp4Config {
        major_brand: str::parse("isom").unwrap(),
        minor_version: 512,
        compatible_brands: vec![
            str::parse("isom").unwrap(),
            str::parse("iso2").unwrap(),
            str::parse("avc1").unwrap(),
            str::parse("mp41").unwrap(),
            str::parse("hev1").unwrap()
        ],        
        timescale: 1000, // This specifies milliseconds
    };

    let dst_file = File::create(output).map_err(|_| "vraw_convert: file creation failed")?;
    let writer = BufWriter::new(dst_file);

    let mut mp4_writer = Mp4Writer::write_start(writer, &config)
        .map_err(|_| "vraw_convert: failed to start writing mp4")?;

    // find first h265 frame
    let mut last_timestamp = 0;
    for entry in &entries {
        let frame =
            parse_raw_frame(&mut f, entry).map_err(|_| "vraw_convert: unable to read frame")?; // we discard the first frame for information about the video media
        match frame.format {
            VideoCaptureFormat::H265 => {
                mp4_writer
                    .add_track(&TrackConfig::from(MediaConfig::HevcConfig(
                        mp4::HevcConfig::default(),
                    )))
                    .map_err(|_| "vraw_convert: failed to add mp4 track")?;

                last_timestamp = frame.timestamp;

                break;
            }
            VideoCaptureFormat::H264 => {
                // Some junk to fulfill H264 requirement for SPS/PPS. VLC corrects for anything we did wrong apparently
                let newsps = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x0a, 0xf8, 0x41, 0xa2];
                let newpps = vec![0x00, 0x00, 0x00, 0x01, 0x68, 0xce, 0x38, 0x80];                
                mp4_writer
                    .add_track(&TrackConfig::from(MediaConfig::AvcConfig(
                        mp4::AvcConfig{width:0, height:0, seq_param_set:newsps, pic_param_set:newpps},
                    )))
                    .map_err(|_| "vraw_convert: failed to add mp4 track")?;

                last_timestamp = frame.timestamp;

                break;
            }            
            VideoCaptureFormat::Stats => {
                continue;
            }
            _ => return Err("VideoCaptureFormat not supported".into()),
        };
    }

    for entry in &entries {
        let raw_frame = parse_raw_frame(&mut f, entry);

        match raw_frame {
            Ok(frame) => {
                if frame.format == VideoCaptureFormat::Stats {
                    continue;
                }

                let delta_t = (frame.timestamp - last_timestamp) as f64 * 1e-6; // duration in milliseconds of the frame
                let video_sample = Mp4Sample {
                    start_time: frame.timestamp as u64,
                    duration: delta_t.round() as u32, // round to nearest millisecond
                    rendering_offset: 0,
                    is_sync: false,
                    bytes: mp4::Bytes::copy_from_slice(frame.raw_data.as_bytes()),
                };

                mp4_writer
                    .write_sample(1, &video_sample)
                    .map_err(|_| "vraw_convert: failed to write sample")?;

                last_timestamp = frame.timestamp;
            }
            Err(_) => {
                // Here, we don't have a valid frame (we most likely reached the end of the recording)
                break;
            }
        }
    }

    mp4_writer
        .write_end()
        .map_err(|_| "vraw_convert: failed to end mp4 writing")?;

    Ok(())
}
