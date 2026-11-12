// C++ helpers the CXX-Qt bridge cannot express on its own.
//
// Three things are missing from `cxx-qt-lib` 0.10 and are provided here:
//
//  * `beginResetModel` / `endResetModel` / `createIndex` / `dataChanged` /
//    `beginInsertRows` / `endInsertRows` — protected members of
//    `QAbstractItemModel`, unreachable from a Rust impl block.
//  * a factory for the package model — CXX-Qt cannot construct a QObject from
//    Rust, so the backend asks C++ to `new` the child model for it.
//
// Nothing here contains application logic; the Rust side owns every decision.
#pragma once

#include <QtCore/QAbstractItemModel>
#include <QtCore/QModelIndex>
#include <QtCore/QObject>

// Full model reset, used when the visible row set changes (filter, search or
// category). Emits `modelAboutToBeReset` / `modelReset`.
void kxbps_model_begin_reset(QAbstractItemModel &model);
void kxbps_model_end_reset(QAbstractItemModel &model);

// A valid index for `row`/`column`, built with the model's own `createIndex`.
QModelIndex kxbps_model_create_index(const QAbstractItemModel &model, int row, int column);

// `dataChanged` for the inclusive row range `[first, last]` and every role.
// Used after an in-place installed/queue snapshot update: the delegates of the
// affected rows re-read their data, while selection and scroll position stay.
void kxbps_model_data_changed(QAbstractItemModel &model, int first, int last);

// Creates the package model as a child of `parent`. The list is empty until
// the first snapshot from xbps arrives.
QObject *kxbps_create_package_model(QObject *parent);

// The queue and log models need the same protected members, but cxx cannot
// declare one extern function in two bridges, so each model has its own name.
void kxbps_queue_model_begin_reset(QAbstractItemModel &model);
void kxbps_queue_model_end_reset(QAbstractItemModel &model);
QModelIndex kxbps_queue_model_create_index(const QAbstractItemModel &model,
                                             int row,
                                             int column);

QModelIndex kxbps_log_model_create_index(const QAbstractItemModel &model,
                                           int row,
                                           int column);
void kxbps_log_model_begin_insert_rows(QAbstractItemModel &model, int first, int last);
void kxbps_log_model_end_insert_rows(QAbstractItemModel &model);

// The mirror model never changes its row set — the rows are the static mirror
// list — so it needs `createIndex` and a per-row `dataChanged`, but no reset.
QModelIndex kxbps_mirror_model_create_index(const QAbstractItemModel &model,
                                              int row,
                                              int column);
void kxbps_mirror_model_data_changed(QAbstractItemModel &model, int first, int last);
