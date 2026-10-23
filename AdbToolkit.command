#!/bin/bash
# Запуск из исходников двойным щелчком в Finder (обычно удобнее готовое приложение из Releases).
cd "$(dirname "$0")" || exit 1
if [ ! -x .venv/bin/python ]; then
  python3 -m venv .venv && .venv/bin/pip install -q -r requirements.txt
fi
exec .venv/bin/python -m adbtoolkit "$@"
