#!/bin/sh
# Fake claw that never terminates on its own — used to test timeout + cancel.
while :; do
    sleep 3600
done
