# Python Environment Manager

## Detect Available Environments

Run all detection probes in sequence and collect results. Store discovered environments in shell variables for later use.

### System Python

```bash
# Check system Python location and version
SYSTEM_PYTHON=$(which python3 2>/dev/null || true)
if [ -n "$SYSTEM_PYTHON" ]; then
  SYSTEM_PYTHON_VERSION=$($SYSTEM_PYTHON --version 2>&1 | awk '{print $2}')
  echo "System Python: $SYSTEM_PYTHON ($SYSTEM_PYTHON_VERSION)"
fi
```

### Active Virtual Environment

```bash
# Check if a virtualenv is currently active
if [ -n "$VIRTUAL_ENV" ]; then
  echo "Active venv: $VIRTUAL_ENV"
  VENV_PYTHON="$VIRTUAL_ENV/bin/python3"
  VENV_VERSION=$($VENV_PYTHON --version 2>&1 | awk '{print $2}')
  echo "  Python: $VENV_VERSION"
fi
```

### Project-Local Virtual Environments

```bash
# Scan common venv directories relative to project root
PROJECT_ROOT="${PROJECT_ROOT:-.}"
for VENV_DIR in "$PROJECT_ROOT/.venv" "$PROJECT_ROOT/venv" "$PROJECT_ROOT/.env" "$PROJECT_ROOT/env"; do
  if [ -x "$VENV_DIR/bin/python3" ]; then
    VER=$("$VENV_DIR/bin/python3" --version 2>&1 | awk '{print $2}')
    echo "Local venv: $VENV_DIR (Python $VER)"
  fi
done
```

### Conda Environments

```bash
# List conda environments (if conda is available)
if command -v conda &>/dev/null; then
  conda env list --json 2>/dev/null | jq -r '.envs[]' | while read -r ENV_PATH; do
    if [ -x "$ENV_PATH/bin/python3" ]; then
      VER=$("$ENV_PATH/bin/python3" --version 2>&1 | awk '{print $2}')
      NAME=$(basename "$ENV_PATH")
      echo "Conda env: $NAME ($ENV_PATH, Python $VER)"
    fi
  done
fi
```

### pyenv

```bash
# List pyenv-managed Python versions
if command -v pyenv &>/dev/null; then
  echo "pyenv versions:"
  pyenv versions --bare 2>/dev/null | while read -r VER; do
    PYENV_BIN="$(pyenv root)/versions/$VER/bin/python3"
    [ -x "$PYENV_BIN" ] && echo "  $VER: $PYENV_BIN"
  done
fi
```

### Poetry

```bash
# Check Poetry environment (must be run from a Poetry project directory)
if command -v poetry &>/dev/null && [ -f "pyproject.toml" ]; then
  POETRY_ENV=$(poetry env info -p 2>/dev/null || true)
  if [ -n "$POETRY_ENV" ]; then
    POETRY_VER=$(poetry env info -e 2>/dev/null | head -1)
    echo "Poetry env: $POETRY_ENV"
  fi
fi
```

### Custom Launchers

Some projects provide their own Python wrapper scripts. Detect common patterns:

```bash
# Isaac Lab launcher
ISAAC_LAUNCHER="$HOME/IsaacLab/isaaclab.sh"
if [ -x "$ISAAC_LAUNCHER" ]; then
  echo "Isaac Lab launcher: $ISAAC_LAUNCHER -p"
  # Get the Python version it wraps
  ISAAC_VER=$("$ISAAC_LAUNCHER" -p -c "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}')" 2>/dev/null || true)
  [ -n "$ISAAC_VER" ] && echo "  Python $ISAAC_VER"
fi

# Generic: check for setup scripts that provide a Python environment
for LAUNCHER in ./setup.sh ./env.sh ./activate.sh; do
  [ -x "$LAUNCHER" ] && echo "Possible launcher: $LAUNCHER (inspect before using)"
done
```

### uv

```bash
# Check if uv is available (fast Python/venv manager)
if command -v uv &>/dev/null; then
  UV_VERSION=$(uv --version 2>&1)
  echo "uv available: $UV_VERSION"
  # List uv-managed Python installations
  uv python list 2>/dev/null || true
fi
```

### Full Discovery Summary

Combine all probes into a single discovery script that outputs a structured summary:

```bash
echo "=== Python Environment Discovery ==="
echo ""

# System
SYSTEM_PY=$(which python3 2>/dev/null || true)
[ -n "$SYSTEM_PY" ] && echo "system: $SYSTEM_PY ($(python3 --version 2>&1 | awk '{print $2}'))"

# Active venv
[ -n "$VIRTUAL_ENV" ] && echo "active-venv: $VIRTUAL_ENV ($($VIRTUAL_ENV/bin/python3 --version 2>&1 | awk '{print $2}'))"

# Local venvs
for d in .venv venv .env env; do
  [ -x "$d/bin/python3" ] && echo "local-venv: $d ($($d/bin/python3 --version 2>&1 | awk '{print $2}'))"
done

# Conda
command -v conda &>/dev/null && echo "conda: $(conda env list --json 2>/dev/null | jq -r '.envs | length') environments"

# pyenv
command -v pyenv &>/dev/null && echo "pyenv: $(pyenv versions --bare 2>/dev/null | wc -l | tr -d ' ') versions"

# uv
command -v uv &>/dev/null && echo "uv: available"

# Poetry
command -v poetry &>/dev/null && [ -f pyproject.toml ] && echo "poetry: $(poetry env info -p 2>/dev/null || echo 'no env')"

echo ""
echo "=== End Discovery ==="
```

## Select the Right Environment

### Decision Rules

Follow this priority order when choosing a Python environment:

| Priority | Condition | Action |
|----------|-----------|--------|
| 1 | Project has a custom launcher (e.g., `isaaclab.sh -p`) | Use the launcher for all project commands |
| 2 | Project has a local `.venv/` or `venv/` | Activate and use it |
| 3 | Project uses Poetry (`pyproject.toml` + `poetry.lock`) | Use `poetry run` or `poetry env info -p` |
| 4 | Project uses Conda (`environment.yml`) | Activate the matching Conda env |
| 5 | Specific Python version required | Use `pyenv` or `uv` to get it, then create a venv |
| 6 | No requirements detected | Use system `python3` in a new venv |

### Check Version Requirements

```bash
# Parse minimum Python version from pyproject.toml
REQUIRED_PYTHON=$(python3 -c "
import tomllib, sys, re
with open('pyproject.toml', 'rb') as f:
    data = tomllib.load(f)
req = data.get('project', {}).get('requires-python', '')
print(req)
" 2>/dev/null || true)

if [ -n "$REQUIRED_PYTHON" ]; then
  echo "Project requires Python $REQUIRED_PYTHON"
fi
```

### Verify an Environment Meets Requirements

```bash
# Compare a candidate Python against a minimum version requirement
CANDIDATE="/path/to/python3"
MIN_MAJOR=3
MIN_MINOR=12

$CANDIDATE -c "
import sys
if sys.version_info >= ($MIN_MAJOR, $MIN_MINOR):
    print(f'OK: Python {sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}')
    sys.exit(0)
else:
    print(f'FAIL: Python {sys.version_info.major}.{sys.version_info.minor} < $MIN_MAJOR.$MIN_MINOR')
    sys.exit(1)
"
```

### Select Between Conflicting Environments

When two tools or sub-projects need different Python versions, do NOT try to share a single environment. Instead:

1. Create separate venvs (see Environment Creation below)
2. Run each tool's commands through its own venv's Python
3. Pass data between them via files (JSON, CSV, pickle), never via shared imports

## Create Environments

### Create a venv with System Python

```bash
ENV_PATH="/path/to/project/.venv"
python3 -m venv "$ENV_PATH"
source "$ENV_PATH/bin/activate"
pip install --upgrade pip
```

### Create a Version-Specific venv

```bash
# Using a specific Python version (must already be installed)
PYTHON_BIN="python3.12"
ENV_PATH="/path/to/project/.venv-3.12"
$PYTHON_BIN -m venv "$ENV_PATH"
source "$ENV_PATH/bin/activate"
pip install --upgrade pip
```

### Create a venv with uv (Preferred When Available)

`uv` is significantly faster than `pip` for venv creation and package installation.

```bash
ENV_PATH="/path/to/project/.venv"

# Create venv (uses the best available Python by default)
uv venv "$ENV_PATH"

# Create venv with a specific Python version (uv will download it if needed)
uv venv "$ENV_PATH" --python 3.12

# Activate
source "$ENV_PATH/bin/activate"
```

### Install a Python Version via pyenv

```bash
# Install a specific version
pyenv install 3.12.8

# Create a venv with it
"$(pyenv root)/versions/3.12.8/bin/python3" -m venv /path/to/env
```

### Install a Python Version via uv

```bash
# Download and install a specific Python version
uv python install 3.12

# Create a venv using it
uv venv /path/to/env --python 3.12
```

### Install Dependencies

```bash
# From requirements.txt
source "$ENV_PATH/bin/activate"
pip install -r requirements.txt

# From pyproject.toml (editable install)
source "$ENV_PATH/bin/activate"
pip install -e .

# With uv (faster, from requirements.txt)
uv pip install -r requirements.txt --python "$ENV_PATH/bin/python3"

# With uv (from pyproject.toml)
uv pip install -e . --python "$ENV_PATH/bin/python3"
```

### Workspace-Scoped Environments

When working within an ATA workspace, place venvs under the run's env directory:

```bash
WID="<workspace_id>"
RUN_ID="<run_id>"
RUN_ROOT=$(ata workspace resolve "@run/$RUN_ID" --workspace "$WID")
ENV_PATH="$RUN_ROOT/env/python-venv"

python3 -m venv "$ENV_PATH"
source "$ENV_PATH/bin/activate"
```

## Run Commands in a Specific Environment

### Using Activation

```bash
# Activate then run
source /path/to/env/bin/activate
python my_script.py
deactivate
```

### Without Activation (Direct Path)

Preferable in scripts and automation -- avoids side effects of activation.

```bash
# Run directly via the venv's Python binary
/path/to/env/bin/python3 my_script.py

# Install packages without activation
/path/to/env/bin/pip install numpy

# Run a module
/path/to/env/bin/python3 -m pytest tests/
```

### Using Custom Launchers

```bash
# Isaac Lab -- always use the launcher for Isaac Lab code
~/IsaacLab/isaaclab.sh -p my_isaac_script.py

# Isaac Lab -- run a module
~/IsaacLab/isaaclab.sh -p -m pytest tests/
```

### Using Poetry

```bash
# Run through Poetry (auto-selects the right env)
poetry run python my_script.py
poetry run pytest tests/
```

### Using Conda

```bash
# Run in a specific Conda environment without activating
conda run -n myenv python my_script.py
conda run -n myenv --no-capture-output python my_script.py
```

## Multi-Environment Workflow

When a project requires multiple Python environments (e.g., Isaac Lab on Python 3.11 + LeRobot on Python 3.12), use isolated venvs and run each tool through its own environment.

### Setup

```bash
PROJECT_ROOT="/path/to/project"

# Environment 1: Isaac Lab (Python 3.11)
ISAAC_ENV="$PROJECT_ROOT/.venv-isaac"
python3.11 -m venv "$ISAAC_ENV"
"$ISAAC_ENV/bin/pip" install --upgrade pip
"$ISAAC_ENV/bin/pip" install -r "$PROJECT_ROOT/isaac-requirements.txt"

# Environment 2: LeRobot (Python 3.12)
LEROBOT_ENV="$PROJECT_ROOT/.venv-lerobot"
python3.12 -m venv "$LEROBOT_ENV"
"$LEROBOT_ENV/bin/pip" install --upgrade pip
"$LEROBOT_ENV/bin/pip" install -r "$PROJECT_ROOT/lerobot-requirements.txt"
```

### Execute in Each Environment

Always use direct-path execution. Never activate one environment inside another.

```bash
# Run Isaac Lab training
"$ISAAC_ENV/bin/python3" train_isaac.py --output /tmp/isaac_output.json

# Run LeRobot inference using Isaac Lab's output
"$LEROBOT_ENV/bin/python3" run_lerobot.py --input /tmp/isaac_output.json
```

### Pass Data Between Environments

Environments cannot share Python imports. Exchange data via files:

| Format | When to use | Example |
|--------|------------|---------|
| JSON | Structured data, configs, small results | `json.dump(data, open("out.json", "w"))` |
| CSV | Tabular data, logs, trajectories | `df.to_csv("out.csv")` |
| NumPy `.npy`/`.npz` | Arrays, tensors, embeddings | `np.save("out.npy", array)` |
| Pickle | Python objects (only between same-version envs) | `pickle.dump(obj, open("out.pkl", "wb"))` |
| Parquet | Large tabular datasets | `df.to_parquet("out.parquet")` |

```bash
# Example: Isaac Lab generates trajectory data, LeRobot consumes it
"$ISAAC_ENV/bin/python3" -c "
import json
trajectory = {'steps': [...], 'metadata': {...}}
with open('/tmp/trajectory.json', 'w') as f:
    json.dump(trajectory, f)
"

"$LEROBOT_ENV/bin/python3" -c "
import json
with open('/tmp/trajectory.json') as f:
    trajectory = json.load(f)
print(f'Loaded {len(trajectory[\"steps\"])} steps')
"
```

### Multi-Env in a Workspace Run

```bash
WID="<workspace_id>"
RUN_ID="<run_id>"
RUN_ROOT=$(ata workspace resolve "@run/$RUN_ID" --workspace "$WID")

# Create both envs under the run's env directory
ISAAC_ENV="$RUN_ROOT/env/isaac"
LEROBOT_ENV="$RUN_ROOT/env/lerobot"

python3.11 -m venv "$ISAAC_ENV"
python3.12 -m venv "$LEROBOT_ENV"

# Shared output directory for cross-environment data
DATA_DIR="$RUN_ROOT/outputs/shared"
mkdir -p "$DATA_DIR"

"$ISAAC_ENV/bin/python3" generate_data.py --output "$DATA_DIR/training_data.json"
"$LEROBOT_ENV/bin/python3" train_model.py --input "$DATA_DIR/training_data.json"
```

## Troubleshooting

### Python Version Not Found

```bash
# Check what Python versions are installed on the system
ls -1 /usr/bin/python3* /usr/local/bin/python3* 2>/dev/null
command -v python3.11 python3.12 python3.13 2>/dev/null

# If a specific version is missing, install via uv or pyenv
uv python install 3.12
# or
pyenv install 3.12.8
```

### venv Creation Fails

```bash
# Ensure the venv module is installed (some distros strip it)
python3 -m ensurepip --default-pip 2>/dev/null || true

# If python3-venv package is missing (Debian/Ubuntu)
# The agent cannot install system packages, so fall back to uv
uv venv /path/to/env --python 3.12
```

### Wrong Python Picked Up

```bash
# Verify which Python is actually running
python3 -c "import sys; print(sys.executable); print(sys.version)"

# Check PATH ordering
which -a python3

# Force a specific Python by using its full path
/usr/local/bin/python3.12 -m venv /path/to/env
```

### Dependency Conflicts Between Environments

If two packages in the same environment have conflicting dependencies:

1. Split into separate environments (one per conflicting package)
2. Run each through its own venv's Python binary
3. Exchange data via files (see Pass Data Between Environments above)

### Stale venv After Python Upgrade

If the system Python was upgraded and an existing venv is broken:

```bash
# Recreate the venv (preserves nothing -- reinstall dependencies after)
rm -rf /path/to/env
python3 -m venv /path/to/env
source /path/to/env/bin/activate
pip install -r requirements.txt
```
