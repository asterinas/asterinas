#!/bin/sh

# usage: corten_benchpsearchy.sh [tc|pt] thread_count [aster_breakdown]

MALLOC=$1
THREAD_COUNT=$2

if [ -z "$THREAD_COUNT" ] || [ "$MALLOC" != "tc" ] && [ "$MALLOC" != "pt" ]; then
    echo "Usage: $0 <tc|pt> <thread_count> [aster_breakdown]"
    exit 1
fi

DO_ASTER_BREAKDOWN=$3

prepare_tdb() {
  TEST_DB=/benchmark/test_db

  if [ -d "$TEST_DB" ]; then
    rm -rf $TEST_DB
  fi
  mkdir -p $TEST_DB

  # Use shell arithmetic to increment i
  i=0
  while [ "$i" -lt "$THREAD_COUNT" ]; do
    dir_path="$TEST_DB/db$i"
    mkdir -p "$dir_path"
    i=$((i + 1))
  done

  echo "Created $THREAD_COUNT directories in $TEST_DB"
}

all_input_files=/benchmark/bin/psearchy/linux_files_index

# Copies all the files in /benchmark/bin/vm_scale_bench_data/linux-2.6.35 to /root/linux-2.6.35
# Which copies from EXT2 FS to RAMFS
copy_files_to_ramfs() {
  SRC_DIR="/benchmark/bin/vm_scale_bench_data/linux-2.6.35"
  DST_DIR="/root"
  
  # Create destination directory if it doesn't exist
  mkdir -p $DST_DIR
  
  # Copy all files from source to destination
  echo "Copying files from $SRC_DIR to $DST_DIR..."
  cp -r $SRC_DIR $DST_DIR/
  
  # Check if the copy was successful
  if [ $? -eq 0 ]; then
    echo "Files copied successfully."
  else
    echo "Error copying files!"
    exit 1
  fi
}

copy_files_to_ramfs

if [ "$MALLOC" == "tc" ]; then
    BIN=/benchmark/bin/psearchy/pedsort-tc
    echo "Using tcmalloc"
else
    BIN=/benchmark/bin/psearchy/pedsort
    echo "Using ptmalloc"
fi

# Run the test
prepare_tdb
echo "***TEST_START***"

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

$BIN -t $TEST_DB/db -c $THREAD_COUNT -m 512 < $all_input_files

if [ "$DO_ASTER_BREAKDOWN" == "aster_breakdown" ]; then
    cat /proc/breakdown-counters
fi

echo "***TEST_END***"
