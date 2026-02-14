# Media Tools

## FFmpeg

[FFmpeg](https://www.ffmpeg.org/) is a complete, cross-platform solution to record, convert and stream audio and video.

### Installation

```nix
environment.systemPackages = pkgs.ffmpeg;
```

### Verified Usage

#### Video and audio processing

```bash
# Create a 10-second blue test video
ffmpeg -f lavfi -i color=blue:duration=10:size=1280x720 -c:v libx264 test.mp4

# Convert to different format
ffmpeg -i test.mp4 test.avi

# Resize video to 640x360
ffmpeg -i test.mp4 -vf scale=640:360 small_test.mp4

# Compress video with lower quality
ffmpeg -i test.mp4 -b:v 500k compressed_test.mp4

# Extract first 5 seconds
ffmpeg -i test.mp4 -t 5 short_test.mp4
```

## ImageMagick

[ImageMagick](http://www.imagemagick.org/) is a software suite to create, edit, compose, or convert bitmap images.

### Installation

```nix
environment.systemPackages = pkgs.imagemagick;
```

### Verified Usage

#### Image processing

```bash
# Create colorful test image
magick -size 1000x600 xc:skyblue -fill white -draw "circle 250,150 250,200" -fill yellow -draw "circle 700,200 700,250" test.jpg

# Convert image format
magick input.jpg output.png

# Resize image
magick input.jpg -resize 800x600 output.jpg

# Crop image
magick input.jpg -crop 400x300+100+50 output.jpg

# Rotate image
magick input.jpg -rotate 90 output.jpg
```

## SoX

[SoX](https://sox.sourceforge.net/) is a sample rate converter for audio files.

### Installation

```nix
environment.systemPackages = pkgs.sox;
```

### Verified Usage

#### Audio processing

```bash
# Create 1 kHz tone (10 seconds)
sox -n test_1k.wav synth 10 sine 1000

# Create chord (multiple tones)
sox -n chord.wav synth 3 sine 261.63 synth 3 sine 329.63 synth 3 sine 392.00

# Create white noise (5 seconds)
sox -n noise.wav synth 5 whitenoise

# Convert audio format
sox input.wav output.flac

# Concatenate audio files
sox file1.wav file2.wav concatenated_output.wav

# Cut audio (from 10s to 40s)
sox input.wav output.wav trim 10 30
```

## Pandoc

[Pandoc](https://hackage.haskell.org/package/pandoc-cli) is a universal document converter.

### Installation

```nix
environment.systemPackages = pkgs.pandoc;
```

### Verified Usage

#### Document conversion

```bash
# Convert Markdown to HTML
pandoc test.md -o test.html

# Convert Markdown to Word DOCX
pandoc test.md -o test.docx

# Convert HTML to Markdown
pandoc test.html -f html -t markdown -o converted.md
```
