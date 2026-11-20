#!/usr/bin/env bash
# probe.sh HOST PORT PATH CODENAME SECONDS: does this Ubuntu mirror serve what apt fetches?
#
# Exit 0 when the mirror answers, 1 when it does not. Used by action.yml in this directory.
#
# Over http, it GETs dists/<suite>/InRelease for <codename>, <codename>-updates and
# <codename>-security, one request each, and reads each body to its end, every request
# bounded by SECONDS. Any one that does not answer 2xx, gives an empty body, or does not
# end in time means the mirror does not answer. A HEAD is not enough: on 19 Nov 2026 a
# runner's mirror answered a HEAD in 1 s and then gave nothing for two InRelease files,
# and the apt-get update after it timed out. An https mirror is judged by a TCP connect.
#
# bash alone (a job container has no curl). About 380 kB on a runner's mirror, three
# requests per job.
set -u
host=$1 port=$2 path=$3 codename=$4 seconds=$5

if [ "$port" = 443 ]; then
  # shellcheck disable=SC2016  # the inner shell's arguments
  timeout -k 2 "$seconds" bash -c 'exec 3<>"/dev/tcp/$1/$2"' _ "$host" "$port" 2>/dev/null
  exit $(( $? == 0 ? 0 : 1 ))
fi

for suite in "$codename" "$codename-updates" "$codename-security"; do
  # shellcheck disable=SC2016  # the inner shell's arguments
  timeout -k 2 "$seconds" bash -c '
    exec 3<>"/dev/tcp/$1/$2" || exit 1
    printf "GET %s/dists/%s/InRelease HTTP/1.0\r\nHost: %s\r\nConnection: close\r\n\r\n" "$3" "$4" "$1" >&3
    IFS= read -r status <&3 || exit 1
    case "$status" in "HTTP/"*" 2"[0-9][0-9]*) ;; *) exit 1 ;; esac
    # The headers end at an empty line; then the body, to the end of the connection.
    cr=$(printf "\r")
    while IFS= read -r line <&3; do [ -z "${line%"$cr"}" ] && break; done
    n=$(wc -c <&3) || exit 1
    [ "$n" -gt 0 ]
  ' _ "$host" "$port" "$path" "$suite" 2>/dev/null || exit 1
done
exit 0
