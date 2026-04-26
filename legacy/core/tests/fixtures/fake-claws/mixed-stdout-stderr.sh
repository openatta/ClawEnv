#!/bin/sh
# Fake claw: emits to both stdout and stderr, verifies stream separation.
echo "out-line-1"
echo "err-line-1" >&2
echo "out-line-2"
echo "err-line-2" >&2
exit 0
