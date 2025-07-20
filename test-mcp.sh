#!/bin/bash

# Test script for Blazing-ART-MCP Server

echo "Testing Blazing-ART-MCP Server..."
echo "================================"

SERVER="./target/release/blazing_art_mcp --entities data/entities.json --events data/events.json"

# Test 1: Initialize
echo -e "\n1. Testing initialization..."
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | $SERVER | head -1

# Test 2: List tools
echo -e "\n2. Testing tools/list..."
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | $SERVER | jq '.result.tools[].name' 2>/dev/null || echo "jq not installed - raw output shown"

# Test 3: Lookup entity
echo -e "\n3. Testing entity lookup (Albert Einstein)..."
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"lookupEntity","arguments":{"name":"Albert Einstein"}}}' | $SERVER | jq '.result.content[0].text' 2>/dev/null || echo "Raw output shown"

# Test 4: Find events
echo -e "\n4. Testing event search (2024-01)..."
echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"findEvents","arguments":{"prefix":"2024-01"}}}' | $SERVER | head -1

# Test 5: Add new entity
echo -e "\n5. Testing add entity (Nikola Tesla)..."
echo '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"addEntity","arguments":{"name":"Nikola Tesla","summary":"Serbian-American inventor and electrical engineer","born":"1856","tags":["inventor","electricity","AC-power"]}}}' | $SERVER | head -1

# Test 6: Add new event
echo -e "\n6. Testing add event..."
echo '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"addEvent","arguments":{"description":"Blazing-ART-MCP server successfully deployed","category":"technology"}}}' | $SERVER | head -1

echo -e "\nAll tests completed!"