#!/bin/bash
# Debug script to check California XXXX courses

echo "=== Checking input CSV for XXXX courses ==="
awk -F',' 'NR>=8 && $3 == "XX" {print "Course",$1,":",$2,"- Prefix:",$3,"Number:",$4}' samples/plans/California_Berkely_V2.csv | head -5

echo ""
echo "=== Checking output metrics for XXXX courses ==="
awk -F',' 'NR>=11 && $3 == "\"XX\"" {print "Course",$1,"- Delay:",$13,"Blocking:",$12,"Complexity:",$11}' output/California_Berkely_V2_w_metrics.csv | head -5

echo ""
echo "=== Total XXXX courses in input ==="
awk -F',' 'NR>=8 && $3 == "XX"' samples/plans/California_Berkely_V2.csv | wc -l

echo ""
echo "=== Total XXXX courses in output ==="
awk -F',' 'NR>=11 && $3 == "\"XX\""' output/California_Berkely_V2_w_metrics.csv | wc -l
