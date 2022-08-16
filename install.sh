#!/bin/sh

set -x
set -e

PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
export PATH

tgt=$HOME/urlharvest/bin

mkdir -p $tgt
cd target/release
rsync -var irssi_urlharvest urllog_meta urllog_generator urllog_actions $tgt/

exit 0
# EOF
