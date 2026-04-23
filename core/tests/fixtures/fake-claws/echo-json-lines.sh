#!/bin/sh
# Fake claw: emits 3 JSON lines to stdout (NDJSON / JsonLines mode).
echo '{"step":"pull","progress":10}'
echo '{"step":"deps","progress":50}'
echo '{"step":"done","progress":100}'
exit 0
