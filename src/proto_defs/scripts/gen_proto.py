import os
from pathlib import Path
import subprocess

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


def _generate(root: Path) -> None:
    if os.getenv("ALICEPI_PROTO_SKIP_BUILD"):
        # Base image already generated the protobuf modules; skip rebuilding.
        return

    proto_dir = root / "src" / "alicepi_proto"
    proto_file = proto_dir / "vad.proto"
    out_dir = root / "src"

    if not proto_file.exists():
        raise FileNotFoundError(f"Proto file not found: {proto_file}")

    cmd = [
        "python",
        "-m",
        "grpc_tools.protoc",
        f"-I{root / 'src'}",
        f"--python_out={out_dir}",
        str(proto_file),
    ]
    subprocess.check_call(cmd, cwd=root)


class BuildHook(BuildHookInterface):
    """Generate protobuf Python modules before packaging."""

    def initialize(self, version, build_data):
        _generate(Path(self.root))


if __name__ == "__main__":
    _generate(Path(__file__).resolve().parents[1])
