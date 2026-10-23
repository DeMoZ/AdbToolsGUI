"""Запуск: python3 -m adbtoolkit. Проверка без окна: python3 -m adbtoolkit --selftest."""
import sys


def main() -> None:
    if "--selftest" in sys.argv:
        from .selftest import run
        sys.exit(run())
    from .gui import main as gui_main
    gui_main()


if __name__ == "__main__":
    main()
