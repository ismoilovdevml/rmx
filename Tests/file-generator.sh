#!/bin/bash

echo "Number of files to generate"
read files

echo "Enter file path:"
read path

if [ ! -d "$path" ]; then
  mkdir -p "$path"
fi

for (( i=0; i <= $files; ++i ))
do
 tmpfile=$(mktemp $path/abc-script.XXXXXXXXXXXXXXXXXXXXXXXXXX)
 dd if=/dev/urandom of=$tmpfile bs=1M count=$(expr 1 + $RANDOM % 3) status=progress
done