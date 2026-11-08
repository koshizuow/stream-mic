#!/bin/bash
# OBS records the "stream-mic" virtual mic the way a streamer's would (pulse_input_capture), next to ffmpeg through
# PulseAudio as the witness; slips.py then checks the two match sample for sample. Needs a running stream-mic with
# sound going through it, obs, Xvfb and ffmpeg. Makes its own OBS profile and scene collection (StreamMicTest).
# Usage: tools/obs_rec_test.sh SECONDS [OUT_DIR] [monitor]
#   monitor: the source also set to "Monitor and Output" into "stream-mic 電腦聲音" (the feedback loop a streamer
#   can make by mistake; stream-mic's loop detection must mute it)
# pw-record is no witness: two of them on the same node slipped 128 to 640 samples against each other and against
# OBS within 90 s, while OBS and ffmpeg (both PulseAudio clients) stayed sample-exact.
set -u
S=${1:-60}
D=${2:-$HOME/work/smp-obs}
MON=0
MONDEV=
if [ "${3:-}" = monitor ]; then MON=2; MONDEV="MonitoringDeviceId=stream-mic-pc-audio
MonitoringDeviceName=stream-mic 電腦聲音"; fi
C=$HOME/.config/obs-studio/basic
mkdir -p "$D" "$C/profiles/StreamMicTest" "$C/scenes"
cat > "$C/profiles/StreamMicTest/basic.ini" <<EOF
[General]
Name=StreamMicTest

[Video]
BaseCX=320
BaseCY=180
OutputCX=320
OutputCY=180
FPSCommon=30

[Audio]
SampleRate=48000
ChannelSetup=Stereo
$MONDEV

[Output]
Mode=Simple

[SimpleOutput]
FilePath=$D
RecQuality=Lossless
EOF
cat > "$C/scenes/StreamMicTest.json" <<EOF
{"name":"StreamMicTest","current_scene":"Scene","current_program_scene":"Scene","scene_order":[{"name":"Scene"}],
 "sources":[
  {"id":"scene","versioned_id":"scene","name":"Scene","uuid":"7d1f0f3e-0000-4000-8000-000000000001","settings":{"items":[{"name":"Mic","source_uuid":"7d1f0f3e-0000-4000-8000-000000000002","visible":true,"id":1}],"id_counter":1}},
  {"id":"pulse_input_capture","versioned_id":"pulse_input_capture","name":"Mic","uuid":"7d1f0f3e-0000-4000-8000-000000000002","settings":{"device_id":"stream-mic"},"mixers":255,"volume":1.0,"muted":false,"monitoring_type":$MON}
 ]}
EOF
rm -f "$D"/*.avi "$D"/obs.wav "$D"/pulse.wav
rm -f ~/.config/obs-studio/.sentinel/run_* 2>/dev/null
Xvfb :78 -screen 0 1280x720x24 >/dev/null 2>&1 &
xv=$!
for _ in $(seq 1 50); do [ -S /tmp/.X11-unix/X78 ] && break; sleep 0.2; done
sleep 0.5
ffmpeg -loglevel error -y -f pulse -sample_rate 48000 -channels 2 -i stream-mic -c:a pcm_f32le "$D/pulse.wav" < /dev/null &
pul=$!
DISPLAY=:78 LIBGL_ALWAYS_SOFTWARE=1 obs --collection StreamMicTest --profile StreamMicTest --scene Scene \
  --disable-updater --disable-shutdown-check --startrecording > "$D/obs.log" 2>&1 &
obs=$!
sleep "$S"
# OBS 32 under Xvfb does not quit on SIGINT; the lossless AVI is readable without its index
kill -INT $obs
for _ in $(seq 1 10); do kill -0 $obs 2>/dev/null || break; sleep 1; done
kill -0 $obs 2>/dev/null && kill -9 $obs
sleep 1
kill -INT $pul
wait $pul 2>/dev/null
kill $xv
f=$(ls -t "$D"/*.avi | head -1)
ffmpeg -loglevel error -y -i "$f" -vn -c:a pcm_f32le "$D/obs.wav" 2>/dev/null
grep -E "pulse-input|audio buffering" "$D/obs.log"
echo "compare: uv run tools/slips.py $D/obs.wav $D/pulse.wav"
