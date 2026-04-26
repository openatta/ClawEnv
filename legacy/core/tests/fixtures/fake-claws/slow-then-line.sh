#!/bin/sh
# Fake claw: sleeps briefly, then emits one JSON line, sleeps again, exits.
# Used to verify streaming (events arrive before process exits).
sleep 0.2
echo '{"step":"first","progress":30}'
sleep 0.2
echo '{"step":"second","progress":90}'
exit 0
