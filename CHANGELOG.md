## 0.3 (2018-10-09)

The main change in this release is the update to the new Tokio API ([link to
the PR](https://github.com/little-dude/rmp-rpc/pull/18)). This is a breaking change:

- The `Service` trait must now implement `Send`
- The signature of `Client::new` and `Endpoint::new` changed: there's not need
  to pass a tokio handle anymore.

Behing the scene, `rmp-rpc` now uses the `tokio` crate instead of `tokio-core`,
but users should be able to keep using `tokio-core`.

### Other changes

- CI now uses `cargo-clippy` instead of re-installing from crate.io
  (ec3f18b1fd683ab4628833f9cd93a607899759db). This should prevent clippy being
  incompatible with the current nightly version
- Update from `tokio_io::codec` to `tokio-codec` (248d5ab5cb6edd453b63e7f627d5bcb225d081a5).
- Fix links in the documentation (ef29105b977d8497ebc3ceda1646a8b19294d872)
- Add a changelog file
