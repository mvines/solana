#!/usr/bin/env bash
#
# Checks if files in the commit range match a regex
#

(
  set -x
  git diff --name-only $TRAVIS_COMMIT_RANGE
)

for file in $(git diff --name-only $TRAVIS_COMMIT_RANGE); do
  if [[ $file =~ ^"$1" ]]; then
    exit 0
  fi
done

exit 1
