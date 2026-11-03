"""Inspect PE manifest resources without executing an installer or triggering UAC."""
import argparse
import ctypes
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def requested_level(path: Path) -> str:
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint32
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    kernel.FreeLibrary.restype = ctypes.c_int
    # LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE: never run DllMain.
    module = kernel.LoadLibraryExW(str(path.resolve(strict=True)), None, 0x22)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        resource = kernel.FindResourceW(module, ctypes.c_void_p(1), ctypes.c_void_p(24))
        if not resource:
            raise RuntimeError(f"{path.name}: manifest resource missing")
        size = kernel.SizeofResource(module, resource)
        loaded = kernel.LoadResource(module, resource)
        data = kernel.LockResource(loaded)
        if not size or not data:
            raise RuntimeError(f"{path.name}: cannot read manifest")
        manifest = ET.fromstring(ctypes.string_at(data, size).rstrip(b"\0"))
        request = manifest.find(".//{*}requestedExecutionLevel")
        if request is None:
            # No explicit elevation request in this x64 application's manifest.
            # Still fails installer verification, which requires an explicit admin level.
            return "asInvoker"
        if request.get("uiAccess", "false").lower() != "false":
            raise RuntimeError(f"{path.name}: unexpected uiAccess")
        return request.get("level", "")
    finally:
        kernel.FreeLibrary(module)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("installers", nargs="+", type=Path)
    parser.add_argument("--application", type=Path, help="also require the app itself to stay asInvoker")
    args = parser.parse_args()
    for path in args.installers:
        level = requested_level(path)
        if level != "requireAdministrator":
            print(f"FAIL {path.name}: expected requireAdministrator, got {level}", file=sys.stderr)
            return 1
        print(f"PASS {path.name}: {level}")
    if args.application:
        level = requested_level(args.application)
        if level != "asInvoker":
            print(f"FAIL {args.application.name}: app must remain asInvoker, got {level}", file=sys.stderr)
            return 1
        print(f"PASS {args.application.name}: {level}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
