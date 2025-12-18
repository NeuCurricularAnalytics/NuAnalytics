#!/usr/bin/env python3
"""
Open Rust documentation in default browser.
Cross-platform: Works on Windows, Linux, and WSL.
"""

import os
import sys
import subprocess
import platform

def is_wsl():
    """Check if running in WSL (Windows Subsystem for Linux)."""
    try:
        with open('/proc/version', 'r') as f:
            return 'microsoft' in f.read().lower() or 'wsl' in f.read().lower()
    except FileNotFoundError:
        return False

def open_in_browser(file_path):
    """Open file in default browser, handling WSL, Linux, and Windows."""
    abs_path = os.path.abspath(file_path)

    if not os.path.exists(abs_path):
        print(f"Error: Documentation file not found: {abs_path}", file=sys.stderr)
        print("Run 'npm run docs' first to generate documentation.", file=sys.stderr)
        sys.exit(1)

    system = platform.system()

    if is_wsl():
        # WSL: Convert to Windows path and use Windows browser
        try:
            result = subprocess.run(
                ['wslpath', '-w', abs_path],
                capture_output=True,
                text=True,
                check=True
            )
            windows_path = result.stdout.strip()
            subprocess.run(['cmd.exe', '/c', 'start', windows_path], check=True)
            print(f"Opening documentation in Windows browser...")
        except subprocess.CalledProcessError as e:
            print(f"Error converting path or opening browser: {e}", file=sys.stderr)
            sys.exit(1)

    elif system == 'Windows':
        # Native Windows
        os.startfile(abs_path) # type: ignore
        print(f"Opening documentation in default browser...")

    elif system == 'Linux':
        # Native Linux
        try:
            subprocess.run(['xdg-open', abs_path], check=True)
            print(f"Opening documentation in default browser...")
        except subprocess.CalledProcessError:
            print(f"Error: Could not open browser. Please open manually: {abs_path}", file=sys.stderr)
            sys.exit(1)

    elif system == 'Darwin':
        # macOS
        subprocess.run(['open', abs_path], check=True)
        print(f"Opening documentation in default browser...")

    else:
        print(f"Unsupported platform: {system}", file=sys.stderr)
        print(f"Please open manually: {abs_path}", file=sys.stderr)
        sys.exit(1)

if __name__ == '__main__':
    doc_path = 'docs/rust/nu_analytics/index.html'
    open_in_browser(doc_path)
