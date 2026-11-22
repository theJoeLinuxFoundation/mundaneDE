"""Record the URI that the guest's desktop launches."""

import os
from pathlib import Path
import sys

(Path(os.environ["XDG_RUNTIME_DIR"]) / "mundane-portal-test/opened").write_text(
    sys.argv[1]
)
