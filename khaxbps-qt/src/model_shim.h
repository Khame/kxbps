// C++ helpers the CXX-Qt bridge cannot express on its own.
//
// Three things are missing from `cxx-qt-lib` 0.10 and are provided here:
//
//  * `QFile` — reading the embedded catalog resource needs `QIODevice`.
//  * `beginResetModel` / `endResetModel` / `createIndex` — protected members of
//    `QAbstractItemModel`, unreachable from a Rust impl block.
//  * a factory for the package model — CXX-Qt cannot construct a QObject from
//    Rust, so the backend asks C++ to `new` the child model for it.
//
// Nothing here contains application logic; the Rust side owns every decision.
#pragma once

#include <QtCore/QAbstractItemModel>
#include <QtCore/QModelIndex>
#include <QtCore/QObject>
#include <QtCore/QString>

// Contents of `path` (a `qrc:` resource), or an empty string when it cannot be
// opened. Decoded as UTF-8; the catalog is ASCII-only JSON.
QString khaxbps_read_resource(const QString &path);

// Full model reset, used when the visible row set changes (filter, search or
// category). Emits `modelAboutToBeReset` / `modelReset`.
void khaxbps_model_begin_reset(QAbstractItemModel &model);
void khaxbps_model_end_reset(QAbstractItemModel &model);

// A valid index for `row`/`column`, built with the model's own `createIndex`.
QModelIndex khaxbps_model_create_index(const QAbstractItemModel &model, int row, int column);

// Creates the package model as a child of `parent`, seeding it with the catalog
// JSON and the installed snapshot (a JSON object of name -> Inst).
QObject *khaxbps_create_package_model(QObject *parent,
                                      const QString &catalogJson,
                                      const QString &installedJson);
