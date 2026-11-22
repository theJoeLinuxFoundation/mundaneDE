import sys
from pathlib import Path
from orca import gsettings_registry, speech_manager

sys.path.insert(0, str(Path.home() / ".local/share/orca"))
registry = gsettings_registry.get_registry()
for key in ("character-echo", "key-echo", "alphabetic-keys"):
    registry.set_runtime_value("typing-echo", key, True)
speech_manager.SPEECH_FACTORY_MODULES[:] = ["recording_speech"]
registry.set_runtime_value("speech", "speech-server-factory", "recording_speech")
