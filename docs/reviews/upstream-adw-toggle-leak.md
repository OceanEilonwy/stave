# Draft upstream report: `AdwToggle` leaks its child

Status: draft, not filed. For the libadwaita issue tracker
(<https://gitlab.gnome.org/GNOME/libadwaita/-/issues>). No existing issue or
merge request about it was found when this was written (November 2026).

---

**Title:** AdwToggle never releases its child widget

**Version:** 1.10.0; the code is the same on `main`.

**What happens:** a widget set as an `AdwToggle`'s `child` is never freed,
even after the toggle and its `AdwToggleGroup` are gone. Everything inside
the child (icons, labels) stays alive for the life of the process.

**Cause** (`src/adw-toggle-group.c` at tag 1.10.0):

- `adw_toggle_set_child` (line 1601) takes a reference:
  `self->child = g_object_ref_sink (child);` (line 1612).
- `AdwToggle` has no `dispose`, and `adw_toggle_finalize` (line 371) frees
  `name`, `label`, `icon_name`, `tooltip` and `description` but not
  `child`, so that reference is never dropped.
- `adw_toggle_group_dispose` (line 1008) unparents each toggle's button
  and drops the toggles; the button lets go of the child, but the toggle's
  own reference remains when the toggle is finalized.

**Fix:** in `adw_toggle_finalize` (or a new `adw_toggle_dispose`), add

```c
g_clear_object (&self->child);
```

**Minimal reproducer** (C):

```c
#include <adwaita.h>

int
main (void)
{
  AdwToggleGroup *group;
  AdwToggle *toggle;
  GtkWidget *label;

  adw_init ();

  group = ADW_TOGGLE_GROUP (g_object_ref_sink (adw_toggle_group_new ()));
  label = gtk_label_new ("Design");
  g_object_add_weak_pointer (G_OBJECT (label), (gpointer *) &label);

  toggle = adw_toggle_new ();
  adw_toggle_set_child (toggle, label);
  adw_toggle_group_add (group, toggle); /* takes the toggle */

  g_object_unref (group); /* group and toggle are finalized */

  g_assert_null (label); /* fails: the label is still alive */

  return 0;
}
```

The same happens with `adw_toggle_group_remove` followed by dropping the
toggle, and with toggles made in a `GtkBuilder` file that use `child`.

**Workaround** for applications: keep a reference to each toggle and, in the
group's `destroy` handler, call `adw_toggle_set_child (toggle, NULL)` before
dropping it.

---

## In Stave

`crates/stave-gtk/src/ui/header.rs` works round it with
`free_children_on_destroy`. The test `libadwaita_leaks_a_toggles_child` there
shows the leak with a plain `adw::ToggleGroup`; when libadwaita is fixed it
fails, and the workaround can go.
