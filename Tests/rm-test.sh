#!/bin/bash

# Check if a directory was provided
if [ "$#" -ne 1 ]; then
    echo "Usage: $0 directory"
    exit 1
fi

DIRECTORY=$1

# Check if the directory exists
if [ ! -d "$DIRECTORY" ]; then
    echo "Directory does not exist: $DIRECTORY"
    exit 2
fi

# Count the number of files in the directory
NUM_FILES=$(find "$DIRECTORY" -type f | wc -l)

if [ "$NUM_FILES" -eq 0 ]; then
    echo "No files to delete in the directory: $DIRECTORY"
    exit 3
fi

# Get the start time
START_TIME=$(date +%s%N)

# Delete the files
rm -f "$DIRECTORY"/*

# Check if the files were deleted
REMAINING_FILES=$(find "$DIRECTORY" -type f | wc -l)

if [ "$REMAINING_FILES" -ne 0 ]; then
    echo "Failed to delete some files: $REMAINING_FILES remaining"
    exit 4
fi

# Get the end time
END_TIME=$(date +%s%N)

# Calculate the time taken in seconds and milliseconds
TIME_TAKEN=$(echo "scale=3;($END_TIME - $START_TIME)/1000000000" | bc)

# Print the results
echo "Deleted files in directory: $DIRECTORY"
echo "Number of files deleted: $NUM_FILES"
echo "Time taken: $TIME_TAKEN s"

exit 0
