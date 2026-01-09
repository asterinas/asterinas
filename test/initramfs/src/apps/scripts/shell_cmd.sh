#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e
set -x

SCRIPT_DIR=/test
cd ${SCRIPT_DIR}

touch hello.txt
mv hello.txt hello_world.txt
rm hello_world.txt

awk '{print $2}' shell_cmd.sh
cp shell_cmd.sh shell_cmd_backup.sh
cat shell_cmd_backup.sh
rm shell_cmd_backup.sh

ln -s shell_cmd.sh tesk_cmd_soft_link
readlink -f tesk_cmd_soft_link
tail -n 1 tesk_cmd_soft_link
rm tesk_cmd_soft_link

ln shell_cmd.sh tesk_cmd_hard_link
tail -n 1 tesk_cmd_hard_link
unlink tesk_cmd_hard_link

sed 3q shell_cmd.sh

find . -name "*shell_cmd*"

mkdir foo
rmdir foo

echo "Hello world from asterinas" > hello.txt
rm hello.txt

cd ..

ps -T | grep ps