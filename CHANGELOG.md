# Changelog

## [0.2.0](https://github.com/pylonsync/pylon/compare/v0.1.0...v0.2.0) (2026-04-24)


### Features

* **cli:** add 'pylon start' — production server command ([a39483e](https://github.com/pylonsync/pylon/commit/a39483e1e8dc7bd96fecd9ff8417cc0f7384e24b))
* **cli:** auto-restart pylon dev on functions/ changes via self-exec ([0247568](https://github.com/pylonsync/pylon/commit/024756853e7660df4bcbee0364bb390e72644fb2))


### Bug Fixes

* **docker:** broken [@pylonsync](https://github.com/pylonsync) symlinks + stale [@pylon](https://github.com/pylon) runtime lookup ([21644f5](https://github.com/pylonsync/pylon/commit/21644f5da25bfae03dcaec3669f5af15637b1767))
* **fly:** drop PYLON_DEV_MODE=false override that fought the Dockerfile default ([6ad04ea](https://github.com/pylonsync/pylon/commit/6ad04ea3bcdf1a2befa484da3faa3bf94ab90b32))
* remove pre-rebrand APIs (AgentDBProvider, @pylon/*, v.money, shard(), etc.) from marketing site; fix pylon-plugin api_keys stale prefix-length test ([7cd4d45](https://github.com/pylonsync/pylon/commit/7cd4d458d43391ea8e63b2863dff5e54f34bdf28))
* **studio:** derive base URL from request Host/X-Forwarded-Proto instead of hardcoded localhost ([704559c](https://github.com/pylonsync/pylon/commit/704559c7506733523083d6e780da306b36738513))
