#!/usr/bin/env bash

set -x
git diff --name-only $TRAVIS_COMMIT_RANGE
for file in $(git diff --name-only $TRAVIS_COMMIT_RANGE); do
  if [[ $file =~ ^web3.js/ ]]; then
    echo $file MATCHES
    exit 0
  fi
  echo NO $file MATCH
done

exit 1
