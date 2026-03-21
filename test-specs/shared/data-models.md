# Shared Data Models — Test Specifications

> **Crate:** `crates/shared/` (`data/`)
> **External Dependencies:** None
> **Existing Tests:** Inline tests in node.rs, interface.rs, and several data files

---

## NodeModel Enum

**What to test:**
- All 60+ variants serialize/deserialize correctly (serde round-trip) `[unit]` **P0**
- Display implementation produces expected strings `[unit]` **P1**
- FromStr parses valid model strings `[unit]` **P1**
- Unknown model string rejected `[unit]` **P0**

---

## NodeState Enum

**What to test:**
- All states serialize/deserialize correctly `[unit]` **P0**
- State values: Unknown, Running, Stopped, etc. `[unit]` **P0**
- Display implementation for terminal output `[unit]` **P1**

---

## NodeKind Enum

**What to test:**
- VirtualMachine, Container, Unikernel serialize/deserialize `[unit]` **P0**
- Kind determines which subsystem handles the node (routing logic) `[unit]` **P1**

---

## NodeConfig Struct

**What to test:**
- Serde round-trip with all 29 fields `[unit]` **P0**
- Default values applied for optional fields `[unit]` **P1**
- Enum fields (OsVariant, CpuArchitecture, MachineType, BiosTypes, etc.) validate correctly `[unit]` **P1**

---

## Interface Types

**What to test:**
- Macro-generated interface enums produce correct names per device model `[unit]` **P0**
- Interface name to index mapping correct per vendor `[unit]` **P0**
- Interface index to name mapping correct per vendor `[unit]` **P0**
- All device models have interface definitions `[unit]` **P1**
- Interface count per model matches data_interface_count `[unit]` **P1**

---

## Lab/Network Types

**What to test:**
- LabInfo, LabSummary, LabStatus serialize/deserialize `[unit]` **P0**
- LabStatus enum: UP, DOWN, PARTIAL, DESTROYED `[unit]` **P0**
- SherpaNetwork, NetworkV4, NetworkV6 types `[unit]` **P1**

---

## Request/Response Types

**What to test:**
- UpRequest/UpResponse round-trip `[unit]` **P0**
- DestroyRequest/DestroyResponse round-trip `[unit]` **P0**
- InspectRequest/InspectResponse round-trip `[unit]` **P0**
- StatusMessage with StatusKind enum `[unit]` **P1**
- All request types include required fields `[unit]` **P0**

---

## User Types

**What to test:**
- User struct with password_hash, is_admin, ssh_keys `[unit]` **P0**
- LoginRequest/LoginResponse round-trip `[unit]` **P0**
- ValidateRequest/ValidateResponse round-trip `[unit]` **P1**

---

## Config Types

**What to test:**
- ClientConfig serialization/deserialization `[unit]` **P0**
- Server Config and Sherpa types `[unit]` **P1**
- TlsConfig paths and skip_verify flag `[unit]` **P1**

**Existing coverage:** Inline tests in node.rs and interface.rs cover some model and interface mapping
