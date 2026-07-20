-- Notify long-running workers whenever URL data changes.

create function notify_url_db_changed()
returns trigger
language plpgsql
as $$
begin
    perform pg_notify('url_db_changed', '');
    return null;
end;
$$;

create trigger notify_url_db_change
after insert or update or delete on url
for each statement
execute function notify_url_db_changed();

create trigger notify_url_meta_db_change
after insert or update or delete on url_meta
for each statement
execute function notify_url_db_changed();

-- url_changed is intentionally retained until older external writers stop
-- updating it. The application no longer reads or updates that table.
