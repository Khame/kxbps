// SPDX-FileCopyrightText: 2026 Łukasz "Khame" Linhard <khame.dev@protonmail.com>
// SPDX-License-Identifier: GPL-3.0-or-later

// Network reachability, as `QNetworkInformation` reports it.
//
// The update check has to tell "the repositories answered and there is nothing
// to install" from "the update list could not be refreshed". xbps cannot say
// which: it prints the same `Input/output error` for a DNS failure, a refused
// connection and a broken repository, and it exits 0 for all three. Qt knows the
// difference, so the check asks it before it starts a process.
//
// `cxx-qt-lib` 0.10 has no binding for `QNetworkInformation`, hence this shim.
// It only reports a number; the meaning of the number is
// `kxbps_core::update_check`'s.
#pragma once

// The platform backend's reachability, as `QNetworkInformation::Reachability`:
// 0 Unknown, 1 Disconnected, 2 Local, 3 Site, 4 Online.
//
// `-1` means no backend could be loaded, which is not the same as "offline":
// the caller treats it as Unknown and lets the network be tried. The backend is
// loaded once and the instance is reused, because loading is not free and the
// answer is read on a worker thread.
int kxbps_network_reachability();
