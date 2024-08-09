#!/bin/sh

set -e

npm run build-extension
cd dist-extension

# Remove development files
rm -r ./dcr*
rm loader.html index.html favicon.ico robots.txt

zip -r ../dist-extension.zip . -x *.DS_Store
