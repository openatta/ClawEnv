#!/bin/sh
# Fake claw: emits one JSON object (JsonFinal mode, like openclaw update --json).
cat <<'EOF'
{
  "status": "success",
  "from_version": "2026.4.1",
  "to_version": "2026.4.5",
  "restart": true
}
EOF
exit 0
