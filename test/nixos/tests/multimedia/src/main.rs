// SPDX-License-Identifier: MPL-2.0

//! The test suite for multimedia applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Audio Processing - SoX
// ============================================================================

#[nixos_test]
fn sox_create_tone(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create 1 kHz tone (10 seconds)
    nixos_shell.run_cmd("sox -n /tmp/test_1k.wav synth 10 sine 1000")?;
    nixos_shell.run_cmd_and_expect("soxi -d /tmp/test_1k.wav", "00:00:10")?;
    Ok(())
}

#[nixos_test]
fn sox_create_chord(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create chord (multiple tones)
    nixos_shell.run_cmd(
        "sox -n /tmp/chord.wav synth 3 sine 261.63 synth 3 sine 329.63 synth 3 sine 392.00",
    )?;
    nixos_shell.run_cmd_and_expect("soxi -d /tmp/chord.wav", "00:00:03")?;
    Ok(())
}

#[nixos_test]
fn sox_create_noise(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create white noise (5 seconds)
    nixos_shell.run_cmd("sox -n /tmp/noise.wav synth 5 whitenoise")?;
    nixos_shell.run_cmd_and_expect("soxi -d /tmp/noise.wav", "00:00:05")?;
    Ok(())
}

#[nixos_test]
fn sox_convert_format(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a wav file and convert to flac
    nixos_shell.run_cmd("sox -n /tmp/input.wav synth 2 sine 440")?;
    nixos_shell.run_cmd("sox /tmp/input.wav /tmp/output.flac")?;
    nixos_shell.run_cmd_and_expect("soxi -t /tmp/output.flac", "flac")?;
    Ok(())
}

#[nixos_test]
fn sox_concatenate_audio(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create two audio files and concatenate
    nixos_shell.run_cmd("sox -n /tmp/file1.wav synth 2 sine 440")?;
    nixos_shell.run_cmd("sox -n /tmp/file2.wav synth 2 sine 880")?;
    nixos_shell.run_cmd("sox /tmp/file1.wav /tmp/file2.wav /tmp/concatenated.wav")?;
    nixos_shell.run_cmd_and_expect("soxi -d /tmp/concatenated.wav", "00:00:04")?;
    Ok(())
}

#[nixos_test]
fn sox_trim_audio(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a 10 second audio file and trim
    nixos_shell.run_cmd("sox -n /tmp/long.wav synth 10 sine 440")?;
    nixos_shell.run_cmd("sox /tmp/long.wav /tmp/trimmed.wav trim 2 5")?;
    nixos_shell.run_cmd_and_expect("soxi -d /tmp/trimmed.wav", "00:00:05")?;
    Ok(())
}

// ============================================================================
// Graphics & Image Editing - ImageMagick
// ============================================================================

#[nixos_test]
fn magick_create_image(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create colorful test image
    nixos_shell.run_cmd(
        "magick -size 1000x600 xc:skyblue -fill white -draw 'circle 250,150 250,200' -fill yellow -draw 'circle 700,200 700,250' /tmp/test.jpg",
    )?;
    nixos_shell.run_cmd_and_expect("identify /tmp/test.jpg", "1000x600")?;
    Ok(())
}

#[nixos_test]
fn magick_convert_image_format(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create an image and convert format
    nixos_shell.run_cmd("magick -size 200x200 xc:red /tmp/input.jpg")?;
    nixos_shell.run_cmd("magick /tmp/input.jpg /tmp/output.png")?;
    nixos_shell.run_cmd_and_expect("identify /tmp/output.png", "200x200")?;
    Ok(())
}

#[nixos_test]
fn magick_resize_image(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create an image and resize
    nixos_shell.run_cmd("magick -size 800x600 xc:blue /tmp/large.jpg")?;
    nixos_shell.run_cmd("magick /tmp/large.jpg -resize 400x300 /tmp/small.jpg")?;
    nixos_shell.run_cmd_and_expect("identify /tmp/small.jpg", "400x300")?;
    Ok(())
}

#[nixos_test]
fn magick_crop_image(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create an image and crop
    nixos_shell.run_cmd("magick -size 400x300 xc:green /tmp/source.jpg")?;
    nixos_shell.run_cmd("magick /tmp/source.jpg -crop 200x150+100+50 /tmp/cropped.jpg")?;
    nixos_shell.run_cmd_and_expect("identify /tmp/cropped.jpg", "200x150")?;
    Ok(())
}

#[nixos_test]
fn magick_rotate_image(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create an image and rotate
    nixos_shell.run_cmd("magick -size 100x200 xc:purple /tmp/vertical.jpg")?;
    nixos_shell.run_cmd("magick /tmp/vertical.jpg -rotate 90 /tmp/horizontal.jpg")?;
    nixos_shell.run_cmd_and_expect("identify /tmp/horizontal.jpg", "200x100")?;
    Ok(())
}

// ============================================================================
// Video Processing - FFmpeg
// ============================================================================

#[nixos_test]
fn ffmpeg_create_video(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a 10-second blue test video
    nixos_shell.run_cmd(
        "ffmpeg -f lavfi -i color=blue:duration=10:size=1280x720 -c:v libx264 /tmp/test.mp4",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ffprobe -loglevel quiet -show_streams -select_streams v:0 -of csv=p=0 /tmp/test.mp4 | grep -q h264 && echo OK",
        "OK",
    )?;
    Ok(())
}

#[nixos_test]
fn ffmpeg_convert_video_format(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a test video and convert to AVI
    nixos_shell.run_cmd(
        "ffmpeg -f lavfi -i color=blue:duration=5:size=640x480 -c:v libx264 /tmp/test2.mp4",
    )?;
    nixos_shell.run_cmd("ffmpeg -i /tmp/test2.mp4 /tmp/test2.avi")?;
    nixos_shell.run_cmd_and_expect(
        "ffprobe -loglevel quiet -show_entries stream=codec_type -select_streams v:0 -of csv=p=0 /tmp/test2.avi | grep -q '^video$' && echo OK",
        "OK",
    )?;
    Ok(())
}

#[nixos_test]
fn ffmpeg_resize_video(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a test video and resize
    nixos_shell.run_cmd(
        "ffmpeg -f lavfi -i color=green:duration=5:size=1280x720 -c:v libx264 /tmp/test3.mp4",
    )?;
    nixos_shell.run_cmd("ffmpeg -i /tmp/test3.mp4 -vf scale=640:360 /tmp/small_test.mp4")?;
    nixos_shell.run_cmd_and_expect("ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 /tmp/small_test.mp4", "640,360")?;
    Ok(())
}

#[nixos_test]
fn ffmpeg_compress_video(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a test video and compress with lower quality
    nixos_shell.run_cmd(
        "ffmpeg -f lavfi -i color=red:duration=5:size=1280x720 -c:v libx264 /tmp/test4.mp4",
    )?;
    nixos_shell.run_cmd("ffmpeg -i /tmp/test4.mp4 -b:v 500k /tmp/compressed_test.mp4")?;
    nixos_shell.run_cmd_and_expect(
        "ffprobe -loglevel quiet -show_streams -select_streams v:0 -of csv=p=0 /tmp/compressed_test.mp4 | grep -q h264 && echo OK",
        "OK",
    )?;
    Ok(())
}

#[nixos_test]
fn ffmpeg_extract_clip(nixos_shell: &mut Session) -> Result<(), Error> {
    // Create a test video and extract first 2 seconds
    nixos_shell.run_cmd(
        "ffmpeg -f lavfi -i color=yellow:duration=10:size=640x480 -c:v libx264 /tmp/test5.mp4",
    )?;
    nixos_shell.run_cmd("ffmpeg -i /tmp/test5.mp4 -t 2 /tmp/short_test.mp4")?;
    nixos_shell.run_cmd_and_expect(
        "ffprobe -loglevel quiet -show_streams -select_streams v:0 -of csv=p=0 /tmp/short_test.mp4 | grep -q h264 && echo OK",
        "OK",
    )?;
    Ok(())
}
