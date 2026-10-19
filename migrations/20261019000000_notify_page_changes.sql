-- Retain the broad channel for metadata processing and existing listeners.
-- The generator only needs changes to data included by its metadata join.
create or replace function notify_url_db_changed()
returns trigger
language plpgsql
as $$
begin
    perform pg_notify('url_db_changed', '');
    if not (tg_table_name = 'url' and tg_op = 'INSERT') then
        perform pg_notify('url_pages_changed', '');
    end if;
    return null;
end;
$$;
