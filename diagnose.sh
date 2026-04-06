#!/bin/bash
# ClawEnv Health Check Diagnostic
# Run: bash diagnose.sh

echo "=========================================="
echo "  ClawEnv Health Check Diagnostic"
echo "=========================================="
echo ""

echo "STEP 1: Is Lima VM running?"
echo "---"
limactl list 2>/dev/null | grep clawenv
echo ""

echo "STEP 2: Can we exec into the VM?"
echo "---"
RESULT=$(limactl shell clawenv-default -- sh -c "echo EXEC_OK" 2>&1)
echo "Direct exec result: '$RESULT'"
echo ""

echo "STEP 3: Is OpenClaw gateway process alive in VM?"
echo "---"
limactl shell clawenv-default -- sh -c "ps aux | grep -E 'openclaw|gateway' | grep -v grep" 2>&1
echo ""

echo "STEP 4: What ports are listening in VM?"
echo "---"
limactl shell clawenv-default -- sh -c "netstat -tlnp 2>/dev/null | grep -E '3000|LISTEN'" 2>&1
echo ""

echo "STEP 5: Can we curl from INSIDE the VM?"
echo "---"
HTTP_CODE=$(limactl shell clawenv-default -- sh -c "curl -s -o /dev/null -w '%{http_code}' --connect-timeout 3 http://127.0.0.1:3000/ 2>/dev/null" 2>&1)
echo "VM curl http://127.0.0.1:3000/ => HTTP $HTTP_CODE"
echo ""

echo "STEP 6: Can we curl from the HOST?"
echo "---"
HOST_CODE=$(curl -s -o /dev/null -w '%{http_code}' --connect-timeout 3 http://127.0.0.1:3000/ 2>/dev/null)
echo "Host curl http://127.0.0.1:3000/ => HTTP $HOST_CODE"
echo ""

echo "STEP 7: Simulate our Rust exec (temp file method)"
echo "---"
# This is EXACTLY what our Rust code does
STAMP=$(date +%s%N)
OUT_FILE="/tmp/.clawenv_exec_${STAMP}"
CMD="curl -s -o /dev/null -w '%{http_code}' --connect-timeout 2 http://127.0.0.1:3000/ 2>/dev/null || echo '000'"
WRAPPER="(${CMD}) > ${OUT_FILE}.out 2> ${OUT_FILE}.err; echo \$? > ${OUT_FILE}.rc"

echo "  Running wrapper command in VM..."
limactl shell clawenv-default -- sh -c "$WRAPPER" 2>&1
echo "  Reading results..."
RC=$(limactl shell clawenv-default -- cat ${OUT_FILE}.rc 2>&1)
OUT=$(limactl shell clawenv-default -- cat ${OUT_FILE}.out 2>&1)
ERR=$(limactl shell clawenv-default -- cat ${OUT_FILE}.err 2>&1)
echo "  rc='$RC' out='$OUT' err='$ERR'"
limactl shell clawenv-default -- sh -c "rm -f ${OUT_FILE}.out ${OUT_FILE}.err ${OUT_FILE}.rc" 2>/dev/null
echo ""

echo "STEP 8: Run health check 5 times (stability test)"
echo "---"
for i in 1 2 3 4 5; do
    S=$(date +%s%N)
    F="/tmp/.diag_${S}"
    limactl shell clawenv-default -- sh -c "(curl -s -o /dev/null -w '%{http_code}' --connect-timeout 2 http://127.0.0.1:3000/ 2>/dev/null || echo '000') > ${F}.out 2> ${F}.err; echo \$? > ${F}.rc" 2>&1
    R=$(limactl shell clawenv-default -- cat ${F}.rc 2>&1)
    O=$(limactl shell clawenv-default -- cat ${F}.out 2>&1)
    limactl shell clawenv-default -- sh -c "rm -f ${F}.out ${F}.err ${F}.rc" 2>/dev/null
    echo "  attempt $i: rc='$R' http_code='$O'"
    sleep 1
done
echo ""

echo "=========================================="
echo "  Diagnostic Complete"
echo "=========================================="
