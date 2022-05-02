-- Create tables and indexes

create table if not exists urllog
(
    id integer primary key autoincrement,
    seen integer not null,
    channel text not null,
    nick text not null,
    url text not null
);
create index urllog_seen on urllog(seen);
create index urllog_channel on urllog(channel);
create index urllog_nick on urllog(nick);
create index urllog_url on urllog(url);

create table urllog_changed
(
    last integer not null
);
insert into urllog_changed values (0);

create table urlmeta
(
    id integer primary key autoincrement,
    url_id integer unique not null,
    lang text,
    title text,
    desc text,
    foreign key(url_id) references urllog(id)
        on update cascade
        on delete cascade
);
create index urlmeta_urlid on urlmeta(url_id);

-- EOF
