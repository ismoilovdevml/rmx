#!/bin/bash

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 directory"
    exit 1
fi

DIRECTORY=$1

if [ ! -d "$DIRECTORY" ]; then
    echo "Directory does not exist: $DIRECTORY"
    exit 2
fi

NUM_FILES=$(find "$DIRECTORY" -type f | wc -l)

if [ "$NUM_FILES" -eq 0 ]; then
    echo "No files to delete in the directory: $DIRECTORY"
    exit 3
fi

START_TIME=$(date +%s)

rm -f "$DIRECTORY"/*

REMAINING_FILES=$(find "$DIRECTORY" -type f | wc -l)

if [ "$REMAINING_FILES" -ne 0 ]; then
    echo "Failed to delete some files: $REMAINING_FILES remaining"
    exit 4
fi

END_TIME=$(date +%s)

TIME_TAKEN=$((END_TIME - START_TIME))

echo "Deleted files in directory: $DIRECTORY"
echo "Number of files deleted: $NUM_FILES"
echo "Time taken: $TIME_TAKEN s"

exit 0
