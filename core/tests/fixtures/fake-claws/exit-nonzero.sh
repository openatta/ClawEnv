#!/bin/sh
# Fake claw: writes to stderr then exits non-zero.
echo "something went wrong" >&2
exit 42
