#!/usr/bin/env sh
set -eu
if [ "${MAIL_ENABLE_INGRESS:-true}" = true ]; then
  export MAIL_INGRESS_BIND=0.0.0.0:8099
fi
exec /usr/local/bin/mail --bind 0.0.0.0:8080 --database /data/mail.sqlite3 --options /data/options.json
