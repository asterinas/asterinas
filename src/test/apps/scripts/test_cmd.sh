#!/bin/sh

set -e
set -x

# FIXME: hardcode script directory here since we do not have pipe
SCRIPT_DIR=/test/scripts
# SCRIPT_DIR=$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )
cd ${SCRIPT_DIR}

touch hello.txt
mv hello.txt hello_world.txt
rm hello_world.txt

awk '{print $2}' test_cmd.sh
cp test_cmd.sh test_cmd_backup.sh
cat test_cmd_backup.sh
rm test_cmd_backup.sh

ln -s test_cmd.sh tesk_cmd_soft_link
readlink -f tesk_cmd_soft_link
tail -n 1 tesk_cmd_soft_link
rm tesk_cmd_soft_link

ln test_cmd.sh tesk_cmd_hard_link
tail -n 1 tesk_cmd_hard_link
unlink tesk_cmd_hard_link

sed 3q test_cmd.sh

find . -name "*test_cmd*" 

mkdir foo
rmdir foo 

echo "Hello world from jinux" > hello.txt
rm hello.txt

cd -
