#!/usr/bin/env fish
begin while true; nc -G 5 -d localhost 60001; echo -en "\r[waiting]" >&2; sleep 1; end; end | defmt-print -w -e target/thumbv7m-none-eabi/debug/aunisoma
