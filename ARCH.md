## Project structure

Monorepo

## Components

device-sdk

## Code structure

#### ferredge-core

edgex-core equivalent
- keeper (being a monolith not necessary)
- data (storage layer with various db abstractions) controlled via feature flags
- metadata (just some structs and related traits-> core logic will be handled by the main app)

### ferredge-sdk

functions to interact with core

### ferredge-ffi

Exported C functions to interact with ferredge. To be built as DLL.

further REF [here](https://doc.rust-lang.org/nomicon/ffi.html))

### ferredge-app

The main application

Core features controlled via feature flags => storage, communication

Each feature should have at least one realted feature flag enabled (viz. storage should have at least one db supported, communication should have at least one protocol supported)

Config options:
- build time config via loading files and env vars at build time (good option to target embedded)
- passed at run time via files and env vars

Dynamic changes in config not targetted initially

## Process mode

Controlled async via feature flags:-
rt => single thread
rt-multi => heavier work-stealing thread

preferably without tokio based pollutions.

## Communication mode

- external communication via supported protocols or ffi
- internal communication via use of Send traits and most probably channels or function calls
- care to be taken to avoid data copies as much as possible

## Storage

- popular time series db options such as timescaledb, influxdb
- embedded options such as sqlite, parquet
