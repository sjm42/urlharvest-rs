# URL harvester for IRC, no bot

* Harvest URLs from irssi IRC client logs, insert into SQL db
* Fetch metadata, e.g. title and update db.
* Generate html pages.
* Implement a search page

Please note: this harvester is tailing your IRC client (irssi) logs on disk and does not need or include an ircbot of any kind.
Consider running your irssi on a cloud vm to stay "always connected" :-)

Any other kind of chat log source would be trivial to implement.
Basically, the chat text is just scanned with regex match and detected URLs are saved & indexed.
