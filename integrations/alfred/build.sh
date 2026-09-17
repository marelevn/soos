#!/bin/sh
# Package the Alfred workflow: copies the app icon in, zips it up as
# soos.alfredworkflow. Run from anywhere; paths are relative to this script.
set -e
cd "$(dirname "$0")"

cp ../../assets/icons/hicolor/256x256.png icon.png
rm -f soos.alfredworkflow
zip -q soos.alfredworkflow info.plist icon.png
rm icon.png

echo "wrote integrations/alfred/soos.alfredworkflow"
