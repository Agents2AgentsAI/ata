---
name: adapt-environment
description: >-
  Adapt code to work with a GPU node's installed environment. Use when an
  experiment fails due to dependency version mismatches, import errors,
  missing packages, or API incompatibilities between the code's expected
  environment and what's actually installed on the GPU node. Triggered by
  errors like 'ModuleNotFoundError', 'ImportError', version conflicts, or
  when the user says 'adapt this code to the environment'.
metadata:
  short-description: Port code to match GPU node dependencies
policy:
  allow_implicit_invocation: true
---

# Adapt Environment

Port code to work with the dependencies installed on a GPU compute node. This skill is used **reactively** — when an experiment fails because the code expects different package versions or APIs than what the node provides.

## When to Use

- Experiment fails with `ModuleNotFoundError`, `ImportError`, or similar
- Version conflicts between code requirements and installed packages
- API changes between expected and installed library versions (e.g., PyTorch, Isaac Sim, TensorFlow)
- Code written for one CUDA version but node has another

## Workflow

### Step 1: Get the environment manifest

Read what's installed on the node:

```
experiment_run_command(<id>, "cat /etc/gpu-env-manifest.txt")
```

This file is baked into the Docker image at build time via `uv pip freeze`. It lists every installed package and version.

### Step 2: Get the error

Read the experiment logs to find the exact failure:

```
experiment_logs(<id>, lines=50)
```

Look for the traceback and the specific import or version error.

### Step 3: Diagnose the mismatch

Compare what the code expects vs what the manifest provides. Common patterns:

| Error pattern | Likely cause | Fix approach |
|---|---|---|
| `ModuleNotFoundError: No module named 'X'` | Package not installed | Add to requirements.txt or find an installed alternative |
| `ImportError: cannot import name 'Y' from 'X'` | API changed between versions | Update import path or use version-appropriate API |
| `RuntimeError: CUDA error` | CUDA version mismatch | Check `nvcc --version` vs code's expected CUDA |
| `AttributeError: module 'torch' has no attribute 'Z'` | PyTorch version difference | Use version-appropriate API call |
| `isaacsim` import errors | Isaac Sim path or version difference | Check installed version, update imports |

### Step 4: Fix the code

Make the minimum changes to the code to work with the installed versions. Priorities:

1. **Pin to what's installed** — don't try to install different versions. The Docker image is the source of truth.
2. **Use version-compatible APIs** — if PyTorch 2.7 removed `torch.old_function()`, find the replacement.
3. **Update requirements.txt** — remove or adjust version pins that conflict with installed packages. If a package is already in the manifest, remove it from requirements.txt entirely (it's pre-installed).
4. **Keep changes minimal** — only change what's broken. Don't refactor unrelated code.

### Step 5: Verify the fix

Re-run the experiment and check logs:

```
experiment_logs(<id>, lines=50)
```

If new errors appear, repeat from Step 2.

## Key Principles

- **Never install conflicting package versions on the node.** The Docker image's environment is carefully curated. Installing a different version of a pre-installed package (e.g., a different PyTorch) can break the entire environment. Instead, adapt the code.
- **The manifest is the contract.** If a package is in `/etc/gpu-env-manifest.txt`, that's the version the code must work with.
- **requirements.txt is for additional packages only.** It should contain packages NOT already in the manifest. Pre-installed packages should be removed from requirements.txt to avoid version conflicts during `uv pip install`.
- **Check CUDA compatibility.** GPU libraries (PyTorch, TensorFlow, JAX) are compiled against specific CUDA versions. The manifest shows which CUDA the environment was built for. Code must match.

## Common Adaptation Patterns

### PyTorch version differences

```python
# Old API (PyTorch < 2.0)
optimizer = torch.optim.Adam(model.parameters(), lr=0.001)
scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=100)

# New API (PyTorch >= 2.0) — compile support
model = torch.compile(model)  # Only available in 2.0+
```

Check installed version: look for `torch==X.Y.Z` in the manifest.

### Isaac Sim / Isaac Lab imports

```python
# Pip-installed Isaac Sim (current environment)
from isaacsim import SimulationApp
import omni.isaac.core

# Old NGC container path (don't use)
# sys.path.insert(0, "/opt/nvidia/isaac-sim/...")
```

Isaac Sim is pip-installed in `/opt/venv`. No path manipulation needed.

### Cleaning requirements.txt

Given manifest contains `torch==2.7.0` and `numpy==1.26.4`:

```txt
# Before (will cause conflicts)
torch>=2.0.0,<2.5.0
numpy==1.25.0
custom-library==1.0.0

# After (only additional packages)
custom-library==1.0.0
```
