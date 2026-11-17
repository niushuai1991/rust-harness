"""Prepare an isolated ZCode profile without changing the user's configuration."""
import argparse
import json
from pathlib import Path
import shutil


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path.home())
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--builtin", type=Path, required=True)
    parser.add_argument("--provider", default="account:bigmodel-individual-coding-plan")
    parser.add_argument("--model", default="GLM-5.3-Flash")
    parser.add_argument("--effort", default="high")
    args = parser.parse_args()
    source = args.source / ".zcode" / "v2"
    destination = args.output / ".zcode" / "v2"
    destination.mkdir(parents=True, exist_ok=False)
    shutil.copy2(source / "credentials.json", destination / "credentials.json")
    config = json.loads((source / "provider_config.json").read_text(encoding="utf-8"))
    config["config"]["defaultModelSelection"] = {"providerId": args.provider, "modelId": args.model,
                                                   "options": {"reasoningLevel": args.effort}}
    (destination / "provider_config.json").write_text(json.dumps(config, ensure_ascii=False), encoding="utf-8")
    shutil.copy2(args.builtin, destination / "builtin.json")


if __name__ == "__main__":
    main()
