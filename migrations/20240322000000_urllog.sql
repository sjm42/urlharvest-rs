-- Create tables and indexes

create table if not exists url
(
    id serial primary key,
    seen bigint not null,
    channel text not null,
    nick text not null,
    url text not null
);
create index url_seen on url(seen);
create index url_channel on url(channel);
create index url_nick on url(nick);
create index url_url on url(url);

create table url_changed
(
    last bigint not null
);
insert into url_changed values (0);

create table url_meta
(
    id serial primary key,
    url_id integer unique not null,
    lang text,
    title text,
    descr text,
    foreign key(url_id) references url(id)
        on update cascade
        on delete cascade
);
create index url_meta_urlid on url_meta(url_id);

-- EOF
