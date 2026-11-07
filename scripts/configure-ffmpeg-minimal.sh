#!/usr/bin/env bash
set -euo pipefail

# Run from an FFmpeg source tree. The course-share downloader only remuxes an
# already-downloaded local HLS playlist (segments + AES keys + init section)
# into MP4 with `-c copy`, so no network, encoder or filter is needed. A few
# decoders stay enabled because avformat_find_stream_info opens them to recover
# video dimensions and audio sample rate from raw TS/mp4 segments.
#
# Usage:
#   ../scripts/configure-ffmpeg-minimal.sh --prefix="$PWD/../prefix" [extra args]

./configure \
  --disable-everything \
  --disable-autodetect \
  --disable-network \
  --disable-doc \
  --disable-avdevice \
  --disable-swscale \
  --disable-swresample \
  --disable-debug \
  --disable-iconv \
  --disable-zlib \
  --disable-bzlib \
  --disable-lzma \
  --disable-sdl2 \
  --disable-x86asm \
  --enable-static \
  --disable-shared \
  --enable-pic \
  --disable-gpl \
  --disable-version3 \
  --disable-nonfree \
  --enable-ffmpeg \
  --disable-ffplay \
  --disable-ffprobe \
  --enable-avfilter \
  --enable-filter=aformat,anull,atrim,crop,format,hflip,null,rotate,transpose,trim,vflip \
  --enable-protocol=file,crypto,data,pipe \
  --enable-demuxer=hls,mpegts,mov,matroska,flv \
  --enable-muxer=mp4,mov,matroska \
  --enable-decoder=h264,hevc,aac,mp3,ac3,eac3,opus,vorbis,flac \
  --enable-parser=aac,h264,hevc,mjpeg,mpeg4video,mpegvideo,vp8,vp9,av1,ac3,opus,vorbis,flac \
  --enable-bsf=aac_adtstoasc,h264_mp4toannexb,hevc_mp4toannexb,extract_extradata \
  "$@"
