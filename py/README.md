# tableverse

High-performance tile-based table viewer for massive datasets. Inspect billions of rows in milliseconds — no sampling, no OOM.

## Install

```bash
pip install tableverse
```

## Usage

```python
import tableverse as tv

# Pandas / Polars / PyArrow — auto-detects type
tv.show(df)

# File path or S3 URI
tv.show("path/to/file.parquet")
tv.show("s3://my-bucket/data/file.parquet")

# Terminal column overview (no browser)
tv.inspect(df)
tv.inspect("path/to/file.parquet")

# DataFrame accessors (pandas and polars)
df.tv.show()
df.tv.inspect()

# Explicit port
tv.show(df, port=8080)
```

## Jupyter

In Jupyter notebooks `tv.show()` automatically renders inline. To use magic commands:

```python
%load_ext tableverse

%tv df                       # show dataframe inline
%tv path/to/file.parquet     # show file inline
%tvinspect df                # print column stats
```

## CLI

```bash
tv file.parquet
tv s3://bucket/data.parquet
tableverse serve file.parquet --port 8080
tableverse inspect file.parquet
tableverse profile file.parquet
```
