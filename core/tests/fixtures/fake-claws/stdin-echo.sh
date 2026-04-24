#!/bin/sh
# Fake claw: reads one line from stdin, echoes it back prefixed with "got: ".
read -r line
echo "got: $line"
exit 0
