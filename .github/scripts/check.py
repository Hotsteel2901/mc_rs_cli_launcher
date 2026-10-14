"""Run a CI check and retain useful diagnostics in GitHub's Checks API annotations.

The normal runner logs remain authoritative. Annotations also make failures
readable to API clients whose networks cannot reach the log artifact host.
"""
import os
import re
import subprocess
import sys


def escape(message):
    return message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


if __name__ == "__main__":
    command = sys.argv[1:]
    result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    print(result.stdout, end="", flush=True)
    if os.environ.get("GITHUB_ACTIONS"):
        title = " ".join(command)
        if result.returncode:
            plain = re.sub(r"\x1b\[[0-9;]*m", "", result.stdout)
            useful = "\n".join(line for line in plain.splitlines() if not line.strip().startswith(("Checking ", "Compiling ", "Downloading ", "Downloaded ")))[-30000:]
            # Individual annotations are truncated by GitHub; publish small numbered chunks.
            for i in range(0, len(useful), 3000):
                print(f"::error title={escape(title)} part {i // 3000 + 1}::{escape(useful[i:i + 3000])}", flush=True)
        else:
            summary = "\n".join(line for line in result.stdout.splitlines() if "test result:" in line or "Finished " in line)
            print(f"::notice title={escape(title)}::{escape(summary or 'Passed')}", flush=True)
    sys.exit(result.returncode)
