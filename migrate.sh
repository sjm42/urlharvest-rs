#!/bin/sh

set -x
set -e

mkdir -p $HOME/urlharvest/html
cd $HOME/urlharvest
mkdir -p data
cd data
cp -v $HOME/urllog/data/urllog2.db url.db
sqlite3 -echo url.db <<EOF
alter table urllog2 rename to url;
alter table urlmeta rename to url_meta;
alter table urllog2_changed rename to url_changed;
EOF

exit 0
# EOF
