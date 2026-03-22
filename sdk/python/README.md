# Tableverse Python SDK

High-performance tile-based table viewer for ML and data engineers.

## Installation

```bash
pip install tableverse
```

## Quick Start

```python
import tableverse as tv
import pandas as pd

df = pd.read_parquet("data.parquet")
tv.show(df)
```

## Integrations

```bash
pip install "tableverse[dagster]"
pip install "tableverse[mlflow]"
pip install "tableverse[dvc]"
pip install "tableverse[spark]"
```

### Dagster

```python
from tableverse.integrations.dagster import TableverseIOManager

defs = Definitions(
    resources={"tv": TableverseIOManager()}
)
```

### MLflow

```python
from tableverse.integrations import mlflow as tv_mlflow
tv_mlflow.show_artifact(run_id, "features.parquet")
```

### DVC

```python
from tableverse.integrations import dvc as tv_dvc
tv_dvc.show("data/features.parquet")
```
