#!/usr/bin/env bash
# Builds the small, audio-only ffmpeg DK.FM downloads (instead of a full 80 MB build): only what
# yt-dlp and DK.FM use — read YouTube/SoundCloud audio (AAC, Opus, Vorbis, MP3, FLAC), save AAC /
# MP3 / FLAC in M4A, MP3, FLAC, OGG files, and turn video thumbnails (WebP, PNG, JPEG) into JPEG.
# Usage: scripts/build-ffmpeg.sh <windows-x64|linux-x64|linux-arm64|darwin-arm64> <out dir>
set -euo pipefail
TARGET=$1
OUT=$(mkdir -p "$2" && cd "$2" && pwd)
FFMPEG=7.1.1
LAME=3.100
WORK=$(pwd)/ffbuild
PREFIX=$WORK/prefix
mkdir -p "$WORK" "$PREFIX"
cd "$WORK"

curl -fsSL --retry 5 -o lame.tar.gz "https://downloads.sourceforge.net/project/lame/lame/$LAME/lame-$LAME.tar.gz"
curl -fsSL --retry 5 -o ffmpeg.tar.xz "https://ffmpeg.org/releases/ffmpeg-$FFMPEG.tar.xz"
tar xzf lame.tar.gz
tar xJf ffmpeg.tar.xz

CROSS=()
LAME_HOST=()
EXE=""
case "$TARGET" in
  windows-x64)
    CROSS=(--target-os=mingw32 --arch=x86_64 --cross-prefix=x86_64-w64-mingw32- --enable-cross-compile --extra-ldflags=-static --pkg-config=pkg-config)
    LAME_HOST=(--host=x86_64-w64-mingw32)
    EXE=".exe"
    ;;
  darwin-arm64)
    LAME_HOST=(--build=aarch64-apple-darwin --host=aarch64-apple-darwin)
    CROSS=(--extra-cflags=-mmacosx-version-min=11.0 --extra-ldflags=-mmacosx-version-min=11.0)
    export MACOSX_DEPLOYMENT_TARGET=11.0
    ;;
  linux-*)
    CROSS=(--extra-ldflags="-static-libgcc")
    ;;
esac

# LAME (MP3 encoder), as a static library
(
  cd "lame-$LAME"
  # (lame 3.100 lists a symbol it doesn't have; harmless for a static library, fatal for some linkers)
  sed -i.bak '/lame_init_old/d' include/libmp3lame.sym
  ./configure "${LAME_HOST[@]}" --prefix="$PREFIX" --enable-static --disable-shared --disable-frontend --disable-decoder --enable-nasm=no
  make -j4 && make install
)

cd "ffmpeg-$FFMPEG"
./configure "${CROSS[@]}" \
  --prefix="$PREFIX" \
  --extra-cflags="-I$PREFIX/include -Os" \
  --extra-ldflags="-L$PREFIX/lib" \
  --enable-small --disable-debug --disable-doc --disable-autodetect --disable-network \
  --disable-ffplay --disable-ffprobe --disable-avdevice --disable-postproc \
  --disable-everything \
  --enable-libmp3lame \
  --enable-protocol=file,pipe \
  --enable-demuxer=mov,matroska,webm_dash_manifest,ogg,mp3,flac,wav,aac,image2,image_webp_pipe,image_png_pipe,image_jpeg_pipe,mjpeg \
  --enable-muxer=ipod,mp4,mov,mp3,flac,wav,ogg,opus,adts,image2,matroska,webm \
  --enable-decoder=aac,aac_latm,opus,vorbis,mp3,mp3float,flac,alac,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,mjpeg,png,webp,vp8 \
  --enable-encoder=aac,libmp3lame,flac,alac,pcm_s16le,mjpeg,png \
  --enable-parser=aac,mpegaudio,flac,opus,vorbis,mjpeg,png,webp,vp8 \
  --enable-bsf=aac_adtstoasc \
  --enable-filter=aresample,aformat,anull,null,format,scale,crop,copy,acopy,volume \
  --enable-swresample --enable-swscale
make -j4
STRIP=strip
[ "$TARGET" = windows-x64 ] && STRIP=x86_64-w64-mingw32-strip
"$STRIP" "ffmpeg$EXE" || true
ls -la "ffmpeg$EXE"
gzip -9 -c "ffmpeg$EXE" > "$OUT/ffmpeg-audio-$TARGET.gz"
ls -la "$OUT"
