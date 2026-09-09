#!/bin/bash
# Setup: Connect Perchance agent to local DecentraAI node as OpenAI-compatible API
# Version: v1.8.0

echo "=== DecentraAI Local API → Perchance Setup ==="
echo ""

# Check if node is running
if curl -s --connect-timeout 2 http://localhost:8080/v1/models > /dev/null 2>&1; then
    echo "✅ Local node is running"
else
    echo "❌ Local node is NOT running. Start it with:"
    echo "   decentraai node --config ~/.decentraai/node.yaml"
    exit 1
fi

# Get the token
TOKEN=$(cat ~/.decentraai/runtime/api.token 2>/dev/null)
if [ -z "$TOKEN" ]; then
    echo "❌ No API token found. Generate one with:"
    echo "   decentraai token create --name perchance-local --tier 2"
    exit 1
fi

echo "✅ API token found: ${TOKEN:0:20}..."
echo ""

# Get available models
MODELS=$(curl -s --connect-timeout 3 -H "Authorization: Bearer $TOKEN" http://localhost:8080/v1/models)
MODEL_COUNT=$(echo "$MODELS" | python3 -c "import json,sys; print(len(json.load(sys.stdin).get('data',[])))" 2>/dev/null || echo "?")
echo "✅ $MODEL_COUNT models available locally"
echo ""

# Perchance configuration instructions
echo "=== PERCHANCE CONFIGURATION ==="
echo ""
echo "In Perchance project (PERCHANCE-PROJECT-SLUG), configure:"
echo ""
echo "  1. Go to Project Settings → AI Plugins"
echo "  2. Add OpenAI-compatible endpoint:"
echo ""
echo "     ┌─────────────────────────────────────────────┐"
echo "     │  URL:  http://localhost:8080/v1/chat/completions"
echo "     │  Key:  $TOKEN"
echo "     │  Model: qwen2.5-3b-instruct-q4_k_m.gguf"
echo "     └─────────────────────────────────────────────┘"
echo ""
echo "  3. In main.pjs, use superFetch to call the local API:"
echo ""
echo "     const response = await superFetch('/v1/chat/completions', {"
echo "       model: 'qwen2.5-3b-instruct-q4_k_m.gguf',"
echo "       messages: [{role: 'user', content: prompt}],"
echo "       max_tokens: 512"
echo "     }, 'http://localhost:8080', {"
echo "       Authorization: 'Bearer ' + '$TOKEN'"
echo "     });"
echo ""
echo "=== TEST COMMANDS ==="
echo ""
echo "  # Test models list:"
echo "  curl -H 'Authorization: Bearer $TOKEN' http://localhost:8080/v1/models"
echo ""
echo "  # Test chat:"
echo "  curl -X POST -H 'Authorization: Bearer $TOKEN' -H 'Content-Type: application/json' \\"
echo "    -d '{\"model\":\"qwen2.5-3b-instruct-q4_k_m.gguf\",\"messages\":[{\"role\":\"user\",\"content\":\"test\"}],\"max_tokens\":16}' \\"
echo "    http://localhost:8080/v1/chat/completions"
echo ""
echo "Done! The Perchance agent can now use your local DecentraAI node."
