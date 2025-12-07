# TODO list

## Table widget

* Drag to resize columns / be smarter about column sizes

## GitHub API service refactor

* Add a way to track when responses are updated (some kind of verisoning?)
  and rebuild the review pane when updated information comes in
* Support paged requests (notifications, issue timeline)
* Be smarter about re-using what's cached on disk

## Inbox

* Load more pages of notifications
* Prefetch thread / PR data
* Show stacked pull requests even when they're read
* Also show issues

## Review view

* Expand / shrink context
* Bookmarks for navigation
* Show time of most recent push / commit (requires database refactor)
* Word wrap / nicer formatting for comments
* Show inline comments
* Allow choosing the base commit

## Review edit

* Add comments / submit review
