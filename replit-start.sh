#!/bin/sh
# Start the Apakabar Archive website: the Run button and the published app both use this.
cd "$(dirname "$0")"
if [ ! -x target/release/apakabar ] || [ ! -f apakabar.db ]; then
  echo "The program or the archive database is missing: run  sh replit-build.sh  in the Shell first."
  exit 1
fi
CONTACT="${APAKABAR_CONTACT:-nath@logosid.app, np2903@tc.columbia.edu}"
# Run through the bundled system libraries when this host lacks the standard program loader.
APAKABAR=./target/release/apakabar
if [ ! -e /lib64/ld-linux-x86-64.so.2 ] && [ -x runtime/ld-linux-x86-64.so.2 ]; then
  APAKABAR="runtime/ld-linux-x86-64.so.2 --library-path $PWD/runtime ./target/release/apakabar"
fi

if [ -n "$REPLIT_DEPLOYMENT" ]; then
  # The published site: reached over HTTPS through Replit's proxy; accounts and editors'
  # decisions must be kept in the PostgreSQL database (files here do not outlast a publish).
  exec $APAKABAR serve --db apakabar.db --addr 0.0.0.0:8080 \
    --secure-cookies --trust-proxy --require-database \
    --contact "$CONTACT" --base-url "${APAKABAR_BASE_URL:-https://apakabar.nathpribady.com}"
fi

# The Run button in the workspace (it uses the development database, if one was added).
if [ -n "$REPLIT_DEV_DOMAIN" ]; then
  exec $APAKABAR serve --db apakabar.db --addr 0.0.0.0:8080 \
    --secure-cookies --trust-proxy --contact "$CONTACT" --base-url "https://$REPLIT_DEV_DOMAIN"
fi
# Anywhere else (your own computer): http://127.0.0.1:8080
exec $APAKABAR serve --db apakabar.db --addr 127.0.0.1:8080 --contact "$CONTACT"
