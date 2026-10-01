#!/bin/bash
S=${AKM_AUDIT_DIR:?export AKM_AUDIT_DIR=<dossier de travail : out/, xdg/, target-uireview/>}
H=$(dirname "$(readlink -f "$0")"); OUT=$S/out/$LABEL; BIN=${BIN:-$S/target-uireview/debug/apihub-app}
ls /dev/hidraw* 2>/dev/null && { echo "hidraw visible, abort"; exit 2; }
python3 $H/fake_services.py 2> $OUT/fake.log & FP=$!
for i in $(seq 50); do grep -q ready $OUT/fake.log && break; sleep 0.1; done
T0=$(date +%s.%N)
if [ "$MODE" = gdb ]; then
  gdb -q -batch -ex 'set pagination off' -ex 'handle SIGUSR1 stop' -ex run -ex 'info threads' -ex 'thread apply 1 bt 30' --args $BIN > $OUT/gdb.txt 2>&1 &
  GP=$!; sleep 1; AP=$(pgrep -n -f "^$BIN"); 
else
  $BIN > $OUT/app.log 2>&1 & AP=$!
fi
echo "pid $AP" > $OUT/meta.txt
for t in $(seq 1 $SECS); do
  sleep 1
  if [ -n "$SHOTS" ] && [[ " $SHOTS " == *" $t "* ]]; then import -display $DISPLAY -window root $OUT/shot_$t.png 2>/dev/null; fi
  W=$(xdotool search --name "Apple Keyboard Monitor" 2>/dev/null | head -1)
  echo "t=$t win=${W:-none} wchan=$(cat /proc/$AP/wchan 2>/dev/null) ticks=$(awk "{print \$14+\$15}" /proc/$AP/stat 2>/dev/null) rss=$(awk "/VmRSS/{print \$2}" /proc/$AP/status 2>/dev/null)" >> $OUT/meta.txt
  [ -n "$ACTION_AT" ] && [ "$t" = "$ACTION_AT" ] && [ -n "$W" ] && eval "$ACTION"
done
import -display $DISPLAY -window root $OUT/final.png 2>/dev/null
if [ "$MODE" = gdb ]; then kill -USR1 $AP; sleep 20; kill -9 $AP 2>/dev/null; wait $GP; else kill $AP 2>/dev/null; sleep 0.5; kill -9 $AP 2>/dev/null; fi
kill $FP 2>/dev/null
echo done >> $OUT/meta.txt
