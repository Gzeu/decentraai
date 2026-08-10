#!/bin/bash
# Production Security Audit Script for DecentraAI
# Run this before deploying to production

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

SCORE=0
TOTAL=10

echo "🔍 Starting Production Security Audit..."
echo ""

# 1. Check system updates
echo "[1/10] Checking system updates..."
if apt list --upgradable 2>/dev/null | grep -q .; then
    echo -e "${YELLOW}⚠️  Security updates available${NC}"
else
    echo -e "${GREEN}✅ System up to date${NC}"
    ((SCORE++))
fi

# 2. Check firewall status
echo "[2/10] Checking firewall..."
if command -v ufw &> /dev/null && ufw status | grep -q "Status: active"; then
    echo -e "${GREEN}✅ Firewall enabled${NC}"
    ((SCORE++))
else
    echo -e "${RED}❌ Firewall disabled${NC}"
fi

# 3. Check SSH configuration
echo "[3/10] Checking SSH hardening..."
if grep -q "PermitRootLogin no" /etc/ssh/sshd_config 2>/dev/null; then
    echo -e "${GREEN}✅ Root SSH disabled${NC}"
    ((SCORE++))
else
    echo -e "${YELLOW}⚠️  Root SSH may be enabled${NC}"
fi

# 4. Check active connections
echo "[4/10] Checking active connections..."
ACTIVE=$(ss -tuln 2>/dev/null | wc -l || echo "0")
if [ "$ACTIVE" -gt 50 ]; then
    echo -e "${YELLOW}⚠️  High number of active connections: $ACTIVE${NC}"
else
    echo -e "${GREEN}✅ Active connections normal: $ACTIVE${NC}"
    ((SCORE++))
fi

# 5. Check disk usage
echo "[5/10] Checking disk usage..."
DISK=$(df / 2>/dev/null | tail -1 | awk '{print $5}' | sed 's/%//' || echo "0")
if [ "$DISK" -gt 80 ]; then
    echo -e "${YELLOW}⚠️  Disk usage high: ${DISK}%${NC}"
else
    echo -e "${GREEN}✅ Disk usage normal: ${DISK}%${NC}"
    ((SCORE++))
fi

# 6. Check memory usage
echo "[6/10] Checking memory usage..."
MEM=$(free 2>/dev/null | grep Mem | awk '{printf("%.0f", $3/$2 * 100.0)}' || echo "0")
if [ "$MEM" -gt 85 ]; then
    echo -e "${YELLOW}⚠️  Memory usage high: ${MEM}%${NC}"
else
    echo -e "${GREEN}✅ Memory usage normal: ${MEM}%${NC}"
    ((SCORE++))
fi

# 7. Check for suspicious processes
echo "[7/10] Checking for suspicious processes..."
if ps aux 2>/dev/null | grep -v grep | grep -qE "(suspicious|malicious|backdoor)"; then
    echo -e "${RED}❌ Suspicious processes detected${NC}"
else
    echo -e "${GREEN}✅ No suspicious processes${NC}"
    ((SCORE++))
fi

# 8. Check SSL certificates
echo "[8/10] Checking SSL certificates..."
if command -v openssl &> /dev/null && openssl s_client -connect localhost:443 -servername localhost 2>/dev/null | openssl x509 -noout -dates 2>/dev/null | grep -q "notAfter"; then
    echo -e "${GREEN}✅ SSL certificate valid${NC}"
    ((SCORE++))
else
    echo -e "${YELLOW}⚠️  SSL certificate check skipped (no HTTPS)${NC}"
fi

# 9. Check API health
echo "[9/10] Checking API health..."
if curl -s -o /dev/null -w "%{http_code}" http://localhost:8000/api/health 2>/dev/null | grep -q "200"; then
    echo -e "${GREEN}✅ API responding${NC}"
    ((SCORE++))
else
    echo -e "${YELLOW}⚠️  API not responding (may not be running)${NC}"
fi

# 10. Check logs for errors
echo "[10/10] Checking error logs..."
ERRORS=$(journalctl --since "24 hours ago" 2>/dev/null | grep -i "error" | wc -l || echo "0")
if [ "$ERRORS" -gt 100 ]; then
    echo -e "${YELLOW}⚠️  High error count: $ERRORS${NC}"
else
    echo -e "${GREEN}✅ Error count normal: $ERRORS${NC}"
    ((SCORE++))
fi

echo ""
echo "================================"
echo "📊 Audit Complete!"
echo "================================"
echo ""
echo "Security Score: ${SCORE} / ${TOTAL}"
echo ""

if [ $SCORE -eq $TOTAL ]; then
    echo -e "${GREEN}🎉 Production Ready! All checks passed.${NC}"
    exit 0
elif [ $SCORE -ge 7 ]; then
    echo -e "${YELLOW}⚠️  Mostly ready, but some issues need attention.${NC}"
    exit 1
else
    echo -e "${RED}❌ Not ready for production. Fix critical issues first.${NC}"
    exit 2
fi
