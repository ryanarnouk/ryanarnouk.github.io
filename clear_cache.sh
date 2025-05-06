#!/bin/bash

# Helper script to clear the cache of both the static resources "static-cache.json" and the regular content files "cache.json"

CACHE="./build/cache.json"
STATIC_CACHE="./build/static-cache.json"

if [ -f "$CACHE_JSON" ]; then
    rm "$CACHE_JSON"
    echo "Cleared $CACHE_JSON"
else
    echo "$CACHE_JSON not found."
fi

if [ -f "$STATIC_CACHE" ]; then
    rm "$STATIC_CACHE"
    echo "Cleared $STATIC_CACHE"
else
    echo "$STATIC_CACHE not found."
fi

echo "Successfully cleared build cache"
