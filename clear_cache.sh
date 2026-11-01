#!/bin/bash

# Helper script to clear the SSG build cache (forces a full rebuild on the next run).
# Equivalent to running the SSG with --force once. Use --clean to also wipe the build folder.

cd "$(dirname "$0")" || exit 1

for CACHE in "./.ssg-cache.json" "./build/cache.json" "./build/static-cache.json"; do
    if [ -f "$CACHE" ]; then
        rm "$CACHE"
        echo "Cleared $CACHE"
    fi
done

echo "Successfully cleared build cache"
