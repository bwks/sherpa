# Sherpad Systemd Service

## Overview

This document describes the systemd service configuration for the Sherpa daemon (sherpad).

The sherpad service manages virtual machines, containers, and unikernels through a unified control plane, providing WebSocket RPC and REST API interfaces for client applications.

## Service Details

- **Service Name**: sherpad.service
- **Service Type**: forking (traditional daemon)
- **User/Group**: sherpa:sherpa
- **Working Directory**: /opt/sherpa
- **Listen Address**: Configurable via sherpa.toml (default: 127.0.0.1:3030)

## File Locations

| File | Location | Purpose |
|------|----------|---------|
| Binary | /opt/sherpa/bin/sherpad | Daemon executable |
| Service File | /etc/systemd/system/sherpad.service | Systemd unit file |
| Environment File | /opt/sherpa/config/sherpad.env | Environment variables (contains password) |
| Environment Example | /opt/sherpa/config/sherpad.env.example | Template for customization |
| PID File | /opt/sherpa/run/sherpad.pid | Process ID file (daemon-managed) |
| Log File | /opt/sherpa/logs/sherpad.log | Application logs |
| Logrotate Config | /etc/logrotate.d/sherpad | Log rotation configuration |

## Dependencies

The sherpad service requires the following system services to be running:

- **docker.service** (Required): Docker daemon for container management
- **libvirtd.service** (Required): Libvirt daemon for VM/unikernel management
- **network-online.target** (Wanted): Network connectivity for WebSocket/API access

If docker or libvirt fail to start, sherpad will not start.

## Management Commands

### Starting/Stopping

```bash
# Start the service
sudo systemctl start sherpad

# Stop the service
sudo systemctl stop sherpad

# Restart the service
sudo systemctl restart sherpad

# Reload configuration (triggers restart)
sudo systemctl reload-or-restart sherpad
```

### Status and Monitoring

```bash
# Check service status (shows lifecycle events)
sudo systemctl status sherpad

# Check if service is running
sudo systemctl is-active sherpad

# Check if service is enabled for boot
sudo systemctl is-enabled sherpad

# Show service properties
sudo systemctl show sherpad
```

### Enable/Disable Auto-start

```bash
# Enable auto-start on boot
sudo systemctl enable sherpad

# Disable auto-start on boot
sudo systemctl disable sherpad

# Enable and start immediately
sudo systemctl enable --now sherpad

# Disable and stop immediately
sudo systemctl disable --now sherpad
```

### Viewing Logs

Since sherpad logs to a file (not systemd journal), use these commands:

```bash
# View entire log file
cat /opt/sherpa/logs/sherpad.log

# View last 50 lines
tail -n 50 /opt/sherpa/logs/sherpad.log

# Follow log in real-time (like tail -f)
tail -f /opt/sherpa/logs/sherpad.log

# View with pager (searchable)
less /opt/sherpa/logs/sherpad.log

# Search logs for errors
grep -i error /opt/sherpa/logs/sherpad.log

# View logs with timestamps
cat /opt/sherpa/logs/sherpad.log | grep "$(date +%Y-%m-%d)"
```

Systemd journal only shows service lifecycle events (start/stop/restart):

```bash
# View systemd service messages only
sudo journalctl -u sherpad

# Follow systemd messages in real-time
sudo journalctl -u sherpad -f

# Show last 100 lines
sudo journalctl -u sherpad -n 100

# Show logs since last boot
sudo journalctl -u sherpad -b
```

## Environment Variables

Environment variables are configured in: `/opt/sherpa/config/sherpad.env`

### Required Variables

- **SHERPA_DB_PASSWORD**: Database password for SurrealDB connection

### Optional Variables

- **RUST_LOG**: Logging level (error, warn, info, debug, trace)
- Additional variables as needed for custom configuration

### Modifying Environment Variables

1. Edit the environment file:
   ```bash
   sudo nano /opt/sherpa/config/sherpad.env
   ```

2. Add or modify variables (one per line):
   ```bash
   SHERPA_DB_PASSWORD=YourPassword
   RUST_LOG=debug
   ```

3. Restart the service to apply changes:
   ```bash
   sudo systemctl restart sherpad
   ```

**Security Note**: The environment file is restricted to sherpa:sherpa with 640 permissions. Do not commit this file to version control.

## Log Rotation

Logs are automatically rotated by logrotate:

- **Frequency**: Daily
- **Retention**: 7 days
- **Compression**: Yes (gzip, delayed by one day)
- **Location**: Old logs stored as `sherpad.log.1.gz`, `sherpad.log.2.gz`, etc.
- **Behavior**: Service is restarted after rotation to reopen log file

### Manual Log Rotation

Force immediate log rotation:

```bash
sudo logrotate -f /etc/logrotate.d/sherpad
```

### Viewing Rotated Logs

```bash
# View compressed log (automatically decompressed)
zcat /opt/sherpa/logs/sherpad.log.1.gz

# Search compressed logs
zgrep "error" /opt/sherpa/logs/sherpad.log.*.gz
```

## Troubleshooting

### Service Won't Start

1. **Check dependencies are running:**
   ```bash
   sudo systemctl status docker
   sudo systemctl status libvirtd
   ```
   
   If either is not running:
   ```bash
   sudo systemctl start docker
   sudo systemctl start libvirtd
   ```

2. **Check for errors in application logs:**
   ```bash
   tail -n 100 /opt/sherpa/logs/sherpad.log
   ```

3. **Check systemd service status:**
   ```bash
   sudo systemctl status sherpad -l
   ```

4. **Verify configuration file exists:**
   ```bash
   ls -la /opt/sherpa/config/sherpa.toml
   cat /opt/sherpa/config/sherpa.toml
   ```

5. **Check environment file:**
   ```bash
   sudo cat /opt/sherpa/config/sherpad.env
   ```

6. **Verify database is running:**
   ```bash
   docker ps | grep sherpa-db
   ```

### Service Keeps Restarting

Check if there's a crash loop:

```bash
sudo systemctl status sherpad
# Look for "Active: activating (auto-restart)"
```

View recent restart history:

```bash
sudo journalctl -u sherpad -n 50
```

If restart limit is hit (5 restarts in 60 seconds):

```bash
# Reset the restart counter
sudo systemctl reset-failed sherpad

# Try starting again
sudo systemctl start sherpad

# Check logs for root cause
tail -f /opt/sherpa/logs/sherpad.log
```

### Port Already in Use

Check what's using the configured port (default 3030):

```bash
# Check port 3030
sudo ss -tlnp | grep 3030

# Or use netstat
sudo netstat -tlnp | grep 3030

# Or use lsof
sudo lsof -i :3030
```

Kill conflicting process or change sherpad port in `/opt/sherpa/config/sherpa.toml`.

### Permission Issues

Verify sherpa user has correct group memberships:

```bash
groups sherpa
# Should show: sherpa libvirt kvm docker
```

If groups are missing:

```bash
# Add to docker group
sudo usermod -aG docker sherpa

# Add to libvirt group
sudo usermod -aG libvirt sherpa

# Add to kvm group
sudo usermod -aG kvm sherpa
```

Check file permissions:

```bash
ls -la /opt/sherpa/bin/sherpad        # Should be 755 sherpa:sherpa
ls -la /opt/sherpa/config/sherpad.env # Should be 640 sherpa:sherpa
ls -la /opt/sherpa/run/               # Should be writable by sherpa
ls -la /opt/sherpa/logs/              # Should be writable by sherpa
```

### Stale PID File

If service won't start due to stale PID file:

```bash
# Check if process is actually running
cat /opt/sherpa/run/sherpad.pid
ps aux | grep $(cat /opt/sherpa/run/sherpad.pid)

# If not running, remove stale PID file
sudo rm /opt/sherpa/run/sherpad.pid

# Try starting again
sudo systemctl start sherpad
```

### Database Connection Issues

Check if SurrealDB container is running:

```bash
docker ps | grep sherpa-db
docker logs sherpa-db
```

Verify database password in environment file:

```bash
sudo cat /opt/sherpa/config/sherpad.env
```

Test database connectivity:

```bash
# From container
docker exec -it sherpa-db /surreal sql --endpoint http://localhost:8000 --namespace sherpa --database sherpa --username sherpa --password "YourPassword"
```

## Security

### Service Hardening

The service includes basic security hardening:

- **NoNewPrivileges=yes**: Prevents privilege escalation
- **PrivateTmp=yes**: Isolated /tmp directory for the service

Additional hardening can be added by editing `/etc/systemd/system/sherpad.service`.

### File Permissions

Recommended file permissions:

```
/opt/sherpa/bin/sherpad          -> 755 (sherpa:sherpa)
/opt/sherpa/config/sherpad.env   -> 640 (sherpa:sherpa) [SENSITIVE]
/opt/sherpa/config/sherpa.toml   -> 644 (sherpa:sherpa)
/opt/sherpa/logs/sherpad.log     -> 640 (sherpa:sherpa)
/opt/sherpa/run/sherpad.pid      -> 644 (sherpa:sherpa)
```

### Environment File Security

The environment file contains sensitive data (database password):

- **Permissions**: 640 (owner read/write, group read, others none)
- **Owner**: sherpa:sherpa
- **Location**: Not in web-accessible directory
- **Backup**: Do not commit to version control

### Service User Isolation

The sherpad service runs as the `sherpa` system user:

- Not a login user (no password)
- Limited to required group memberships (sherpa, libvirt, kvm, docker)
- Home directory: /opt/sherpa
- No sudo access

## Uninstallation

To remove the systemd service:

```bash
# Stop the service
sudo systemctl stop sherpad

# Disable auto-start
sudo systemctl disable sherpad

# Remove service file
sudo rm /etc/systemd/system/sherpad.service

# Reload systemd
sudo systemctl daemon-reload

# Reset failed states
sudo systemctl reset-failed

# Remove logrotate config
sudo rm /etc/logrotate.d/sherpad
```

Or use the uninstall script:

```bash
cd /path/to/sherpa
sudo ./scripts/sherpa_uninstall.sh --remove-all
```

## Customization

### Changing Listen Address/Port

1. Edit configuration file:
   ```bash
   sudo nano /opt/sherpa/config/sherpa.toml
   ```

2. Update `server_ipv4` and `server_port` values:
   ```toml
   server_ipv4 = "0.0.0.0"  # Listen on all interfaces
   server_port = 3030
   ```

3. Restart service:
   ```bash
   sudo systemctl restart sherpad
   ```

### Adding Custom Environment Variables

1. Edit environment file:
   ```bash
   sudo nano /opt/sherpa/config/sherpad.env
   ```

2. Add your variables (one per line):
   ```bash
   MY_CUSTOM_VAR=value
   ANOTHER_VAR=another_value
   ```

3. Restart service:
   ```bash
   sudo systemctl restart sherpad
   ```

### Modifying Service Behavior

1. Edit service file:
   ```bash
   sudo nano /etc/systemd/system/sherpad.service
   ```

2. Make changes (e.g., add resource limits, change restart policy)

3. Reload systemd and restart service:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl restart sherpad
   ```

### Changing Log Rotation Policy

1. Edit logrotate config:
   ```bash
   sudo nano /etc/logrotate.d/sherpad
   ```

2. Modify settings (e.g., change `rotate 7` to `rotate 30` for 30 days retention)

3. No restart needed (logrotate runs automatically via cron)

## Integration Notes

### With Docker

- Service waits for Docker to be ready before starting (`After=docker.service`)
- Sherpa user must be in docker group (handled by installation)
- Access to Docker socket: `/var/run/docker.sock`
- Container operations performed via Bollard Rust library

### With Libvirt

- Service waits for libvirtd to be ready before starting (`After=libvirtd.service`)
- Sherpa user must be in libvirt and kvm groups (handled by installation)
- Access to libvirt socket: `/var/run/libvirt/libvirt-sock`
- VM/unikernel operations via libvirt API

### With SurrealDB

- Database container (sherpa-db) should be running
- Connection details in `/opt/sherpa/config/sherpa.toml`
- Password in `/opt/sherpa/config/sherpad.env`
- Default connection: localhost:8000

## Performance Tuning

### Resource Limits

Current limits (can be adjusted in service file):

```ini
LimitNOFILE=65536     # Maximum open files
TasksMax=4096         # Maximum number of tasks
```

To adjust:

1. Edit service file:
   ```bash
   sudo nano /etc/systemd/system/sherpad.service
   ```

2. Modify or add limits:
   ```ini
   LimitNOFILE=131072
   LimitNPROC=4096
   MemoryLimit=4G
   CPUQuota=200%
   ```

3. Reload and restart:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl restart sherpad
   ```

### Monitoring Resource Usage

```bash
# Show current resource usage
sudo systemctl status sherpad

# Show detailed resource usage
sudo systemd-cgtop

# Show memory usage
sudo systemctl show sherpad | grep Memory

# Show CPU usage
sudo systemctl show sherpad | grep CPU
```

## References

- [Systemd service documentation](https://www.freedesktop.org/software/systemd/man/systemd.service.html)
- [Systemd unit file format](https://www.freedesktop.org/software/systemd/man/systemd.unit.html)
- [Logrotate documentation](https://linux.die.net/man/8/logrotate)
- [Sherpa installation guide](../scripts/sherpa_install.sh)
- [Sherpa GitHub repository](https://github.com/bwks/sherpa)

## Support

If you encounter issues not covered in this document:

1. Check application logs: `tail -f /opt/sherpa/logs/sherpad.log`
2. Check systemd logs: `sudo journalctl -u sherpad -f`
3. Review configuration: `/opt/sherpa/config/sherpa.toml`
4. Report issues: https://github.com/bwks/sherpa/issues
