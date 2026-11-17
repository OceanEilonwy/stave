# Test coverage of the language spec

This maps every rule of [`docs/grammar.md`](grammar.md) (draft 3), section by
section, and every code of its Diagnostics table, to the tests that pin it
down. A rule is **covered** when a test asserts its precise outcome (solved
sizes and positions, evaluated values, resolved names, or the exact
diagnostic). **partial** means part of the rule is tested and the rest is
pending, and **pending** names the step that will make it testable.

`crates/stave-app/tests/coverage.rs` enforces this file: every row has a
status, every covered or partial row links at least one test, every link
points at a test (or broken design) that exists, and every code of the
Diagnostics table has a row whose status agrees with the broken-design
corpus. A new diagnostic code can't be added to the spec without a decision
here.

Links are written `crate/file::test` for
`crates/stave-<crate>/tests/<file>.rs` (or `src/<file>.rs` for unit tests),
and `broken/<folder>` for a design in `tests/broken/`.

## Features

### Lexical grammar

| Rule | Tests | Status |
|---|---|---|
| Spaces, tabs and comments separate tokens; comments are kept but never parsed | `lang/lexer::comments_and_docs`, `lang/spec_syntax::comments_are_kept_but_never_parsed` | covered |
| Block comments don't nest | `lang/spec_syntax::block_comments_do_not_nest` | covered |
| An unclosed block comment is reported where it starts | `lang/spec_syntax::an_unclosed_block_comment_is_reported_at_its_start` | covered |
| A byte order mark is whitespace | `lang/lexer::a_byte_order_mark_is_whitespace` | covered |
| Every byte is in some token (the lexer covers the input) | `lang/lexer::every_byte_is_covered`, `lang/corpus::the_tree_is_lossless` | covered |
| The keyword list | `lang/lib::keywords_match_the_grammar`, `lang/syntax::keyword_kinds_match_the_keyword_list`, `lang/ast::an_ident_is_never_a_keyword` | covered |
| A keyword where a new name is expected is `reserved-word` | `lang/parser::a_keyword_where_a_new_name_goes_is_reserved`, `broken/reserved_word` | covered |
| A keyword is a `NAME` after `.` and before `:` | `lang/parser::keywords_are_names_after_a_dot_and_before_a_colon`, `lang/ast::a_name_may_be_a_keyword` | covered |
| Face and direction words, types and built-in names are not keywords | `app/features::a_part_may_be_called_like_a_face_word`, `app/spec_names::a_builtin_is_found_only_when_called` | covered |
| Contextual words are special in one position only | `lang/lib::contextual_words_match_the_grammar`, `lang/parser::contextual_words_in_their_positions`, `lang/parser::contextual_words_are_ordinary_names_elsewhere` | covered |
| A quantity is one token; the longest quantity wins (the disambiguation table) | `lang/lexer::quantity_forms`, `lang/lexer::the_disambiguation_table`, `app/spec_values::a_mixed_fraction_is_one_quantity`, `app/spec_values::a_length_minus_a_length_written_without_spaces_is_two_quantities`, `app/spec_values::a_count_minus_a_length_is_a_unit_mismatch` | covered |
| Values are exact fractions (`1/3in`) | `app/spec_values::a_third_of_an_inch_is_exact`, `lang/units::a_third_of_an_inch_is_a_valid_literal`, `lang/units::rationals_stay_exact` | covered |
| The units `mm cm m in ft deg` | `app/spec_values::every_unit_converts_exactly`, `app/spec_values::angles_add_like_lengths`, `lang/lib::units_match_the_grammar`, `lang/units::mixed_units_agree` | covered |
| A number followed by letters that aren't a unit is `unknown-unit` | `broken/unknown_unit`, `lang/parser::one_mistake_one_error`, `lang/spec_syntax::every_syntax_error_has_a_help_line` | covered |
| Digits may be grouped with `_` | `app/spec_values::digits_may_be_grouped_with_underscores` | covered |
| A fraction's denominator can't be zero | `lang/parser::one_mistake_one_error` | covered |
| A literal too large is an error, never a wrapped value | `lang/units::huge_literals_overflow_instead_of_wrapping` | covered |
| Newlines are dropped inside `( )` and `[ ]` | `lang/lexer::newlines_are_dropped_inside_brackets` | covered |
| Newlines are dropped after the continuation tokens | `lang/lexer::newlines_are_dropped_after_continuation_tokens`, `lang/lib::line_continuation_tokens_match_the_grammar`, `lang/syntax::continuation_kinds_match_the_grammar_list` | covered |
| Nothing else drops a newline; a field ends at its line | `lang/lexer::newlines_are_kept_everywhere_else`, `lang/parser::a_field_ends_at_its_line`, `lang/diagnostics::an_operator_at_the_start_of_a_line` | covered |
| `within` suppresses the newline after it | `app/spec_values::a_newline_after_within_continues_the_line` | covered |
| The newline before an item keyword or a doc comment is never dropped | `lang/spec_syntax::a_line_ending_in_an_operator_does_not_swallow_the_next_declaration`, `lang/spec_syntax::a_doc_comment_still_attaches_across_its_newline` | covered |
| A missing value is reported on its own line | `lang/parser::a_missing_value_is_reported_on_its_own_line`, `broken/missing_value` | covered |
| An item keyword at the start of a line closes open brackets | `lang/lexer::an_item_keyword_closes_open_brackets`, `lang/parser::an_unclosed_bracket_costs_one_error`, `broken/unclosed_bracket` | covered |
| A keyword followed by `:` (an argument name) doesn't close brackets | `lang/lexer::a_keyword_used_as_an_argument_name_does_not_close_brackets`, `lang/spec_syntax::a_keyword_argument_name_at_the_start_of_a_line_keeps_brackets_open` | covered |
| A doc comment line closes open brackets | `app/one_mistake::a_bracket_forgotten_before_a_doc_comment_is_one_error` | covered |
| `}` and the end of the file close open brackets, with a note saying where | `lang/lexer::a_closing_brace_and_the_end_of_file_close_brackets`, `lang/spec_syntax::a_closing_brace_closes_brackets_left_open_inside_its_block`, `lang/spec_syntax::the_end_of_the_file_closes_brackets_still_open` | covered |
| A bracket is closed where one line ends with a value and the next starts another | `lang/spec_syntax::a_bracket_is_closed_where_two_values_meet_across_a_line_break`, `app/one_mistake::a_bracket_forgotten_in_a_place_block_is_one_error_on_its_line` | covered |
| A missing `}` ends the body at the next item it can't contain | `lang/parser::a_missing_brace_ends_the_body_at_the_next_item`, `lang/parser::a_missing_brace_inside_a_component_body`, `lang/spec_syntax::a_brace_missing_at_the_end_of_the_file`, `broken/missing_brace` | covered |
| Doc comments attach to the next item | `lang/spec_syntax::doc_comments_attach_to_the_next_item`, `lang/parser::a_missing_brace_ends_the_body_at_the_next_item` | covered |
| In a kind, doc comments may precede a param, a require, a makes or a member | `lang/spec_syntax::in_a_kind_doc_comments_may_precede_a_param_a_require_or_a_makes`, `lang/spec_syntax::in_a_hardware_kind_doc_comments_may_precede_a_member` | covered |
| A doc comment anywhere else is `syntax` | `lang/spec_syntax::a_doc_comment_anywhere_else_is_a_syntax_error`, `lang/parser::one_mistake_one_error` | covered |
| Strings split into text and interpolation parts, each with its span | `lang/parser::interpolation_parts_have_spans`, `lang/lexer::strings_split_into_parts_with_spans` | covered |
| Interpolation only in `note:`, `check:` and require messages | `lang/parser::interpolation_is_allowed_only_in_notes_checks_and_require_messages` | covered |
| Interpolated values are filled in, lengths in the project's units | `app/spec_values::a_require_message_fills_in_values_in_the_projects_units`, `app/spec_joints::a_kind_note_fills_in_its_roles` | covered |
| Names inside strings resolve like any other name | `app/spec_values::an_interpolated_name_must_resolve` | covered |
| The escapes `\" \\ \n \t \{ \u{...}`, and nothing else | `lang/spec_syntax::a_string_with_every_escape_parses_to_its_text`, `lang/spec_syntax::escapes_are_a_closed_list`, `app/spec_values::escapes_in_strings`, `lang/lexer::string_errors` | covered |
| A string ends on its line | `lang/spec_syntax::a_string_ends_on_its_line` | covered |

### Syntactic grammar

| Rule | Tests | Status |
|---|---|---|
| `block(X)`: entries separated by newlines or commas, optional at both ends | `lang/spec_syntax::block_entries_are_separated_by_newlines_or_commas` | covered |
| An empty block is fine | `lang/spec_syntax::an_empty_block_is_fine` | covered |
| A file starts with its header; imports come before items | `lang/parser::a_missing_header_is_reported_and_items_still_parse`, `lang/parser::imports_come_first` | covered |
| The header's format version is 1 | `app/spec_names::a_file_in_another_format_version_is_not_supported` | covered |
| Every item form parses | `lang/parser::every_item_form_parses`, `lang/ast::items_know_their_keyword_tag_and_name` | covered |
| Kind and recipe entries: `roles`, `cut`, `makes`, `mark` vs fields | `lang/parser::contextual_words_in_their_positions` | covered |
| Two tokens of lookahead: a word followed by `:` is always a field or argument name | `lang/parser::contextual_words_in_their_positions`, `lang/parser::keywords_are_names_after_a_dot_and_before_a_colon` | covered |
| Positional arguments come before named ones | `lang/parser::one_mistake_one_error` | covered |
| At most one `place`; a joint has no body; a hardware body holds only `place` | `lang/parser::one_mistake_one_error` | covered |
| `part x = ...` is always a mirror image; `mirror` takes a source and `about:` | `lang/parser::one_mistake_one_error` | covered |
| An array holds labels when two or more names, otherwise a count | `app/spec_names::a_single_name_in_brackets_is_a_count_not_a_label`, `lang/ast::labels_need_two_distinct_names`, `lang/parser::one_mistake_one_error` | covered |
| `[same]` only when `same` is directly followed by `]` | `lang/parser::contextual_words_in_their_positions` | covered |
| Precedence and associativity of operators | `lang/parser::precedence_and_associativity`, `lang/ast::operators_know_their_precedence`, `app/spec_values::multiplication_binds_tighter_than_addition` | covered |
| Comparisons don't chain | `lang/parser::one_mistake_one_error`, `app/spec_values::comparisons_do_not_chain` | covered |
| There are no object literals: `{ }` only after a declaration header | `lang/parser::one_mistake_one_error` | covered |
| Expressions nested too deep (brackets or operator chains) are an error, not a crash | `lang/parser::deep_nesting_is_an_error_not_a_crash`, `lang/fuzz::pathological_inputs_finish`, `app/fuzz::the_deepest_expression_the_parser_accepts_checks_without_overflowing` | covered |
| The patch grammar | `lang/parser::patch_files_parse`, `lang/format::formats_patch_files` | covered |

### Names

| Rule | Tests | Status |
|---|---|---|
| The namespace of a word is decided by where it stands | `app/features::a_part_may_be_called_like_a_face_word`, `app/features::enum_words_win_where_they_are_expected` | covered |
| A name declared twice in a module, in any of its files, is `duplicate-name` | `app/features::unknown_names_and_duplicates_are_reported`, `app/features::names_are_shared_by_every_file_of_a_module`, `broken/duplicate_name` | covered |
| A name declared twice in a component body is `duplicate-name` | `app/spec_names::a_name_declared_twice_in_a_component_is_a_duplicate_name` | covered |
| An inner scope may reuse an outer name; the inner one wins | `app/spec_names::an_inner_scope_may_reuse_an_outer_name_and_wins`, `app/features::components_are_instantiated_per_assembly_with_names_resolved_where_written` | covered |
| Params, roles and made parts of a kind share one scope | `app/spec_joints::roles_params_and_made_parts_of_a_kind_share_one_namespace` | covered |
| The prelude is the outermost scope; a module item may take a prelude name | `app/spec_names::a_module_item_may_take_the_name_of_a_prelude_item_without_a_warning` | covered |
| A kind hiding a kind, or a recipe hiding a recipe, is `shadows-prelude` | `app/features::a_kind_hiding_the_prelude_is_a_warning`, `app/spec_names::a_recipe_hiding_a_standard_recipe_is_a_warning`, `broken/shadows_prelude` | covered |
| Built-in functions are looked up only when called | `app/features::a_builtin_and_a_param_of_the_same_name_coexist`, `app/spec_names::a_builtin_is_found_only_when_called` | covered |
| A kind or component can't take a built-in function's name | `app/spec_names::a_kind_or_component_can_not_take_a_builtin_functions_name` | covered |
| Enum words win where they are expected | `app/features::enum_words_win_where_they_are_expected` | covered |
| A position that accepts both readings is `ambiguous-name` | `app/features::a_word_and_an_item_both_accepted_are_ambiguous`, `broken/ambiguous_name` | covered |
| Members never clash with items | `app/spec_names::members_never_clash_with_items` | covered |
| Names are resolved where they are written | `app/spec_names::a_library_component_sees_its_own_module_not_the_users`, `app/features::components_are_instantiated_per_assembly_with_names_resolved_where_written` | covered |
| A module name that is a keyword or not a word needs `as` | `modules/memory::keyword_folder_without_alias_is_reserved_word`, `modules/memory::folder_that_is_not_a_word_is_reserved_word`, `modules/memory::std_hardware_without_alias_is_reserved_word` | covered |
| A module name can't equal an item name of the importing module | `modules/memory::import_name_clashing_with_an_item` | covered |
| An unknown name suggests the closest names | `app/spec_names::an_unknown_name_suggests_the_closest_names`, `app/suggestions::typos_get_the_closest_names`, `app/suggestions::qualified_names_suggest_the_module_s_items` | covered |
| A module's name alone is not a value | `app/spec_names::a_module_name_alone_is_not_a_value` | covered |

### Arrays

| Rule | Tests | Status |
|---|---|---|
| A counted array is numbered from 0 | `app/spec_names::a_counted_array_is_numbered_from_zero`, `app/features::a_label_is_not_an_item_and_numbers_index_counted_arrays` | covered |
| The count is a constant or a param with a `within` list | `app/spec_names::the_count_is_a_constant_or_a_param_with_a_within_list`, `app/spec_names::a_count_with_a_unit_is_one_error` | covered |
| A labelled array is indexed by label only | `app/features::a_label_is_not_an_item_and_numbers_index_counted_arrays`, `app/features::labelled_arrays_are_indexed_by_label_and_same_is_elementwise` | covered |
| A counted array is indexed by a count expression | `app/spec_names::a_counted_array_is_indexed_by_a_count_not_a_label`, `app/spec_names::a_counted_array_index_may_be_an_expression` | covered |
| Only arrays can be indexed | `app/spec_names::only_arrays_can_be_indexed` | covered |
| `x[*]` is every element, and flattens inside a list | `app/spec_names::star_flattens_inside_a_list`, `app/spec_declarations::scene_fields_show_hide_explode_and_section` | covered |
| `x[same]` is elementwise, for labels or counts | `app/features::labelled_arrays_are_indexed_by_label_and_same_is_elementwise`, `app/spec_names::same_pairs_counted_arrays_of_the_same_count` | covered |
| `[same]` needs the same labels in the same order, or the same count | `app/features::same_needs_matching_labels`, `app/spec_names::same_needs_the_labels_in_the_same_order`, `app/spec_names::same_pairs_counted_arrays_of_the_same_count` | covered |
| `[same]` outside an array declaration is an error | `app/spec_names::same_is_only_allowed_inside_an_array_declaration` | covered |
| The same label twice is `duplicate-name`, from the parser | `app/spec_names::the_same_label_twice_is_a_duplicate_name` | covered |
| An unindexed array name in its own declaration is the element | `app/spec_names::an_unindexed_array_name_in_its_own_declaration_is_the_element` | covered |
| Anywhere else an unindexed array name is `type-mismatch` | `app/features::an_unindexed_array_name_needs_star` | covered |

### Modules and imports

| Rule | Tests | Status |
|---|---|---|
| A folder is a module; its files share names | `app/features::names_are_shared_by_every_file_of_a_module`, `modules/memory::root_project_with_two_files` | covered |
| File names and order don't matter; splitting or merging files never changes meaning | `cli/determinism::the_order_files_are_listed_in_does_not_change_anything`, `cli/determinism::the_order_files_are_created_on_disk_in_does_not_change_anything`, `cli/determinism::merging_a_modules_files_into_one_does_not_change_the_design` | covered |
| Subfolders are separate modules | `modules/disk::clothing_cabinet` | covered |
| Exactly one `project` or `library` item per module | `modules/memory::module_without_project_or_library`, `modules/memory::module_with_two_headers`, `app/one_mistake::a_syntax_error_in_the_project_block_is_one_error` | covered |
| A folder is imported whole; its items are used qualified | `app/features::modules_are_used_qualified_and_private_items_stay_inside`, `modules/memory::import_with_alias` | covered |
| The module name is the folder's last segment or the name after `as` | `modules/memory::import_with_alias`, `modules/memory::explicit_std_import_with_alias` | covered |
| An import applies to the file it is written in | `app/spec_names::an_import_applies_to_the_file_it_is_written_in` | covered |
| Imports can't form a cycle | `modules/memory::cycle_between_two_modules`, `modules/memory::self_import_is_a_cycle`, `broken/import_cycle` | covered |
| Relative-path sources | `modules/disk::joint_samples_imports_japanese_joinery`, `modules/memory::same_folder_from_two_files_is_loaded_once` | covered |
| `std:` sources | `modules/memory::explicit_std_import_with_alias`, `modules/memory::unknown_std_module`, `modules/memory::root_identical_to_std_module_is_that_module` | covered |
| A missing folder is `unknown-module` | `modules/memory::missing_folder_is_unknown_module`, `broken/unknown_module` | covered |
| Remote sources parse and are `not-supported` | `modules/memory::remote_source_is_not_supported`, `broken/not_supported` | covered |
| `stave.lock`, fetching on request, remote patches need confirmation | — | pending: remote modules are a later step (plan.md, "Modules") |
| A library holds no part, assembly, region, joint, hardware or scene at its top level | `app/spec_names::every_kind_of_placed_item_is_not_in_a_library`, `broken/not_in_library` | covered |
| Inside a component a library may hold them | `app/spec_names::a_library_may_hold_placed_items_inside_a_component` | covered |
| Items are public unless `private` | `app/features::modules_are_used_qualified_and_private_items_stay_inside` | covered |
| Placement stays inside a module | `app/spec_names::a_library_component_can_not_reach_the_parts_of_the_module_using_it` | covered |
| A patch file is not part of its folder's module | `modules/memory::patch_file_is_not_part_of_the_module` | covered |

### The prelude

| Rule | Tests | Status |
|---|---|---|
| Every public item of `std:joinery`, `std:hardware` and `std:tools` is in scope everywhere | `app/spec_names::the_prelude_provides_every_item_the_grammar_lists`, `modules/memory::std_is_always_loaded_in_prelude_order` | covered |
| The standard modules are `.stave` libraries that check clean and don't clash | `app/pipeline::every_example_and_standard_module_checks_clean`, `model/corpus::every_module_resolves_cleanly` | covered |
| An explicit import reaches a prelude item a local name hides | `app/spec_names::a_hidden_prelude_item_is_reached_by_importing_its_module` | covered |

### Declarations and their fields

| Rule | Tests | Status |
|---|---|---|
| An unknown field is `unknown-field`, listing the fields; a misspelt required field is not also missing | `app/spec_declarations::an_unknown_project_field_lists_the_fields`, `app/features::a_misspelt_field_is_one_error_not_two`, `broken/unknown_field` | covered |
| A missing required field is `missing-field` | `app/spec_declarations::a_part_needs_length_width_and_place`, `broken/missing_field` | covered |
| A field written twice is `duplicate-name` | `app/spec_declarations::a_field_written_twice_is_a_duplicate_name` | covered |
| `project`: goal, units (default mm), resolution (default 0.5mm), style, tools (left out: all), methods | `app/spec_declarations::project_fields_have_their_defaults`, `app/spec_declarations::project_fields_are_read` | covered |
| `project` field types: units are length units, resolution a length, tools tools, methods recipes | `app/spec_declarations::output_units_are_lengths_only`, `app/spec_declarations::project_resolution_is_a_length`, `app/spec_declarations::a_misspelt_tool_is_an_unknown_name`, `app/spec_declarations::project_methods_are_recipes` | covered |
| `library`: about, license; the body is optional | `app/spec_declarations::library_fields`, `app/spec_declarations::a_library_body_is_optional` | covered |
| `param`: a type, a default, or both, and they agree | `app/spec_values::a_param_takes_its_type_from_its_default`, `app/spec_values::a_param_with_a_type_and_a_default_must_agree`, `lang/ast::a_param_has_a_type_or_a_default` | covered |
| A module param must have a default; a component or kind param without one must be given | `app/spec_values::a_param_at_the_top_level_of_a_module_must_have_a_default`, `app/features::a_component_param_without_a_default_must_be_given`, `app/spec_joints::a_kind_param_without_a_default_must_be_given` | covered |
| A default may refer to other params (any order), items, and in a kind its roles | `app/spec_values::param_defaults_may_refer_to_each_other_in_any_order`, `app/features::joint_kinds_are_evaluated_per_pair` | covered |
| A value worked out from itself is `type-mismatch` | `app/spec_values::a_value_worked_out_from_itself_is_a_type_mismatch` | covered |
| `within` gives the allowed values, of the param's type | `app/spec_values::within_values_must_have_the_params_type`, `app/spec_values::a_module_params_default_must_lie_within_its_range` | covered |
| An argument outside a component or kind param's `within` is `out-of-range` | `app/features::values_outside_within_are_out_of_range`, `app/spec_joints::a_setting_outside_the_kinds_within_is_out_of_range`, `broken/out_of_range` | covered |
| `locked` parses and is kept | `app/spec_values::locked_is_written_last`, `lang/parser::every_item_form_parses` | partial: smart edits, which `locked` stops, are a later step |
| A module param's `within` is the range the design must work over | — | pending: checking over a range needs z3 (layer 6) |
| A misspelt type name is `unknown-name` | `app/spec_values::a_misspelt_type_name_is_an_unknown_name` | covered |
| `let`: a named value, used before or after it is written, not passed in | `app/spec_values::a_let_is_a_named_value_used_before_or_after_it_is_written`, `app/spec_values::a_let_can_not_be_passed_into_a_component` | covered |
| `stock`: species, form and thickness required; species and forms are word lists | `app/spec_declarations::stock_needs_species_form_and_thickness`, `app/spec_declarations::species_and_forms_are_closed_word_lists`, `schema/species::species_match_the_grammar`, `schema/words::stock_forms_match_the_grammar` | covered |
| Dowel stock is sized by diameter; only dowels have one | `app/spec_declarations::dowel_stock_is_sized_by_diameter` | covered |
| Every stock field is readable | `app/spec_declarations::every_stock_field_is_readable` | covered |
| Parts cut from stock with a `width:` take that width | `app/spec_declarations::a_part_cut_from_stock_with_a_width_takes_that_width` | covered |
| A part whose cut length is over the stock's `max_length` is `type-mismatch` | `app/spec_declarations::a_part_longer_than_its_stocks_max_length_is_a_type_mismatch` | covered |
| Parts wider than `max_width` are planned as glue-ups | `app/spec_declarations::a_wider_part_than_max_width_is_planned_as_a_glue_up` | partial: no error, as the spec says; the glue-up plan is the work plan's (a later step) |
| `part`: length, width (unless the stock's or a dowel) and place required | `app/spec_declarations::a_part_needs_length_width_and_place`, `app/spec_declarations::only_round_parts_have_a_diameter` | covered |
| A part is cut from a stock | `app/spec_declarations::a_part_is_cut_from_a_stock` | covered |
| `thickness`: thinner than the stock, never thicker | `app/spec_declarations::a_part_may_be_thinner_than_its_stock_but_not_thicker` | covered |
| Sizes must be more than zero | `app/spec_declarations::a_size_must_be_more_than_zero` | covered |
| `grain`: length (default) or width | `app/spec_declarations::grain_runs_along_the_length_by_default` | covered |
| `face_side` and `face_edge` are face words across the thickness and width | `app/spec_declarations::face_side_and_face_edge_are_face_words`, `app/spec_declarations::the_reference_face_must_lie_across_the_parts_thickness` | covered |
| `note` is kept for the work card | `app/spec_declarations::face_side_and_face_edge_are_face_words`, `app/spec_values::escapes_in_strings` | covered |
| Readable on every part: sizes, `stock`, the six faces, `diameter` on round parts; not grain or shape | `app/spec_declarations::every_part_reads_its_sizes_stock_and_faces`, `app/spec_declarations::only_round_parts_have_a_diameter`, `app/spec_declarations::grain_shape_and_reference_faces_are_not_readable` | covered |
| Only a part's length or width may be `fit` | `app/spec_declarations::only_length_and_width_may_be_fit` | covered |
| `region`: no fields; readable faces and `across`, `up`, `deep` | `app/spec_placement::a_region_reads_its_faces_and_sizes` | covered |
| `assembly`: the component's params as fields; its items and params readable | `app/spec_declarations::an_assembly_reads_its_components_params_and_items`, `app/features::a_component_param_without_a_default_must_be_given`, `app/spec_declarations::an_assembly_is_an_instance_of_a_component` | covered |
| An array of assemblies | `app/spec_declarations::an_array_of_assemblies_takes_its_arguments_per_element` | covered |
| `profile`: measure required, smooth optional, two or more stations; readable length | `app/spec_declarations::a_profile_needs_a_measure_and_two_stations`, `app/spec_declarations::profile_entries_are_typed`, `app/spec_declarations::a_turned_part_is_as_long_as_its_profile` | covered |
| Radius stations are doubled to diameters | `app/spec_declarations::radius_stations_are_doubled_to_diameters` | covered |
| Stations must not decrease, used or not; equal stations make a shoulder | `app/spec_declarations::stations_must_not_decrease_even_in_an_unused_profile`, `app/spec_declarations::two_stations_at_one_distance_make_a_shoulder` | covered |
| `smooth: true` draws one curve through the stations | — | pending: the curve is drawn by stave-geom's turned meshes; not asserted here |
| `scene`: pose, show, hide, explode, section | `app/spec_declarations::scene_fields_show_hide_explode_and_section`, `app/spec_declarations::scene_fields_are_typed` | covered |
| `require name : condition "message"`; the condition is a Bool | `app/spec_declarations::a_require_condition_is_a_bool`, `app/spec_declarations::a_require_needs_its_message` | covered |
| `kind ... : joint` entries (roles, param, require, insert, cut_first, adds, uses, cut, makes, note) | see [Joint definitions](#joint-definitions) | covered |
| Every joint call accepts `glue:` (default true) and `methods:`; a kind can't declare them | `app/spec_joints::every_joint_accepts_glue_and_methods`, `app/spec_joints::a_kind_can_not_declare_glue_or_methods` | covered |
| Hardware kinds: required entries per base, as a field or a param | `app/spec_joints::a_hardware_kind_needs_its_bases_required_entries`, `app/spec_joints::a_required_entry_may_be_a_param_and_every_entry_is_readable` | covered |
| Hardware kinds have no roles, cuts, makes or joint fields | `app/spec_joints::hardware_kinds_have_no_roles_cuts_or_joint_fields` | covered |
| A hinge kind is a cup hinge or a butt hinge, never both, never neither | `app/features::a_hinge_kind_is_a_cup_hinge_or_a_butt_hinge` | covered |
| Entries of a slide, hinge, pull or fastener kind are readable | `app/spec_joints::a_required_entry_may_be_a_param_and_every_entry_is_readable`, `app/features::kind_values_carry_settings_and_are_readable`, `app/spec_joints::a_pulls_screw_diameter_defaults_to_4mm` | covered |
| A tool kind's `title` and `powered` | `app/spec_joints::a_tool_kind_has_a_title_and_powered`, `app/spec_declarations::project_fields_are_read` | partial: a tool's entries are not readable in expressions yet (a tool is a value without its kind's entries) |
| `recipe`: tools and cut required; rank, needs, mark, check | `app/spec_joints::a_recipe_needs_tools_and_cutting_steps`, `app/spec_joints::recipe_fields_are_typed`, `app/spec_joints::a_recipe_uses_every_step_feature_and_need_of_its_operation` | covered |

### Placement

| Rule | Tests | Status |
|---|---|---|
| Directions, axes and faces; faces of the drawn box | `lang/placement::faces_and_directions_agree`, `lang/placement::axes_know_the_others`, `app/spec_placement::against_puts_the_opposite_face_on_the_anchor` | covered |
| A part's own axes length, width, thickness | `app/spec_placement::orient_names_where_the_length_and_width_run`, `app/spec_placement::without_orient_a_part_lies_flat` | covered |
| Anchors are faces or mid-planes of parts and regions | `app/spec_placement::an_anchor_is_a_face_of_a_part_or_region`, `app/spec_placement::center_on_a_region_along_a_world_axis` | covered |
| `ground` fixes all three axes; it may be turned with `orient` | `app/spec_placement::the_ground_part_may_be_turned_with_orient`, `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| The floor is wherever the lowest part ends up | `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| `against(face)`: the opposite face touches | `app/spec_placement::against_puts_the_opposite_face_on_the_anchor`, `app/features::against_and_flush_with_gaps_and_insets` | covered |
| `against(face, gap: d)`: moved `d` outward from the face | `app/spec_placement::against_with_gap_is_measured_outward_from_the_anchor_face`, `app/features::against_and_flush_with_gaps_and_insets` | covered |
| `flush(face)` and `flush(face, inset: d)`: same-named face, moved into the anchor's part | `app/spec_placement::flush_with_inset_moves_into_the_anchors_part`, `app/features::against_and_flush_with_gaps_and_insets` | covered |
| `against` and `flush` take a face, not a mid-plane; the anchor's axis is the axis fixed | `app/spec_placement::against_and_flush_take_a_face_not_a_mid_plane`, `broken/type_mismatch`, `lang/placement::terms_fix_the_axis_of_their_anchor` | covered |
| `center(item, axis)` | `app/spec_placement::center_on_a_region_along_a_world_axis`, `app/features::center_on_an_item_and_between_planes` | covered |
| `center(from:, to:)` between two parallel anchors | `app/features::center_on_an_item_and_between_planes`, `app/spec_placement::center_between_needs_two_parallel_anchors` | covered |
| `spaced`: arrays only, in declaration order, between parallel anchors | `app/spec_placement::spaced_needs_an_array_and_two_parallel_anchors`, `app/spec_placement::spaced_runs_from_a_mid_plane_too` | covered |
| Spacing `evenly` | `app/features::a_label_is_not_an_item_and_numbers_index_counted_arrays` | covered |
| Spacing `ends` and `ends(inset:)` | `app/spec_placement::spaced_ends_puts_the_first_and_last_element_flush_with_the_anchors`, `app/features::spaced_pitch_and_gap` | covered |
| Spacing `pitch(d)` and `gap(d)` | `app/features::spaced_pitch_and_gap` | covered |
| With `from` after `to`, elements still run from `from` | `app/features::spaced_pitch_and_gap` | covered |
| With `pitch` or `gap`, an element past `to` is `overlap` | `broken/pitch_runs_past_end`, `app/fit::a_pitch_that_runs_past_the_end` | covered |
| `corners(part, inset:)`: exactly the four corner labels | `app/features::corners_place_the_four_labels`, `app/spec_placement::corners_needs_the_four_corner_labels`, `lang/placement::corners_are_two_flush_terms` | covered |
| `orient`: length and width perpendicular | `app/features::orient_turns_a_part_and_needs_perpendicular_directions`, `lang/placement::orientation_must_be_perpendicular` | covered |
| `width:` may be left out only for a round part; it then runs right, or back | `app/spec_placement::orient_needs_a_width_unless_the_part_is_round`, `lang/placement::a_round_part_needs_only_its_length` | covered |
| Without `orient` a part lies flat | `app/spec_placement::without_orient_a_part_lies_flat` | covered |
| At most one `orient` | `app/spec_placement::orient_is_written_once` | covered |
| Placement terms are a fixed vocabulary | `app/spec_placement::a_placement_term_is_one_of_a_fixed_vocabulary`, `schema/placement::placement_terms_match_the_grammar` | covered |
| Rule 1: exactly one part is `ground`; never inside a component | `app/features::exactly_one_part_is_ground`, `app/spec_placement::ground_is_never_inside_a_component`, `broken/ground` | covered |
| Rule 2: anchors belong to things placed earlier in dependency order; a cycle is `placement-cycle` | `app/spec_placement::parts_are_placed_in_dependency_order_not_text_order`, `app/features::parts_placed_against_each_other_are_a_cycle`, `app/spec_placement::a_longer_placement_cycle_lists_every_part`, `broken/placement_cycle` | covered |
| Rule 3: each axis fixed by exactly one term | `app/features::every_axis_is_set_exactly_once`, `broken/axis_not_set`, `broken/axis_set_twice`, `lang/placement::a_placement_has_one_term_per_axis` | covered |
| Rule 4: joints never position parts | `app/spec_placement::joints_never_position_parts` | covered |
| A part a joint makes is placed by the kind | `app/spec_joints::a_made_part_is_named_after_its_joint_and_made_once_per_pair` | covered |
| Hardware is mounted on a face of its piece; `face:` names it | `app/spec_placement::face_names_another_mount_face`, `schema/kinds::mount_faces_match_the_grammar` | covered |
| A pull mounts on its piece's front by default; place sets the face's two axes | `app/spec_placement::a_pull_is_mounted_on_the_front_face_of_its_piece_by_default`, `app/features::hardware_on_its_mount_face` | covered |
| The mount face fixes the axis it is normal to | `app/spec_placement::the_mount_face_fixes_the_axis_it_is_normal_to`, `lang/placement::hardware_terms_stay_off_the_mount_face_axis` | covered |
| A hinge's place sets only the axis along its piece's length | `app/spec_placement::a_hinge_is_placed_along_its_pieces_length_only` | covered |
| A slide is fixed by its base: a `place` is `axis-set-twice` | `app/spec_placement::a_slide_is_fixed_entirely_by_its_base` | covered |
| A slide mounts on the side nearer its opening's side, front ends level, centred on the width | `app/spec_placement::a_slide_mounts_on_the_side_nearer_its_openings_side`, `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| `orient`, `ground` and `corners` don't apply to hardware | `app/spec_placement::orient_ground_and_corners_do_not_apply_to_hardware` | covered |
| A cup hinge is a cup, a plate, an arm and two links; the plate and arm stay on the inside face of `on`, the links turn about their arm pins | `app/pipeline::the_clothing_cabinet_is_where_the_design_says`, `app/spec_joints::a_cup_hinge_bores_its_cup_in_the_mount_face`, `app/view::a_cup_hinge_cup_is_described_as_deep`, `scene/cabinet::the_hinge_links_follow_the_door` | covered |
| A butt hinge is two leaves, housed in mortises at the pin edge | `app/features::butt_hinge_leaves_are_housed_in_both_parts_at_the_flush_edge`, `app/features::a_mirrored_butt_hinged_door_cuts_its_mortise_in_the_other_side`, `app/view::a_butt_hinge_is_two_leaves` | covered |
| A butt hinge's mount face must lie against `on` | `app/features::a_butt_hinge_needs_its_parts_to_meet_at_the_mount_face`, `broken/butt_hinge_meets_nothing` | covered |
| A slide with a box envelope is two members | `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| A mirrored part: same stock and sizes, reflected position and frame | `app/spec_placement::a_mirrored_part_has_its_sources_stock_sizes_and_the_reflected_position`, `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| A mirror is about a plane | `app/spec_placement::a_mirror_is_about_a_plane` | covered |
| A mirrored assembly keeps its item names and re-binds outside items | `app/features::mirrored_assemblies_rebind_outside_parts`, `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| Nothing at the reflected place is `mirror` | `app/features::a_mirror_without_a_counterpart_is_an_error`, `broken/mirror` | covered |
| The cut list marks a mirrored part; the work plan marks the pair together | — | pending: the cut list and work plan are later steps (stave-output, stave-workplan) |

### Sizes

| Rule | Tests | Status |
|---|---|---|
| The drawn size is what the field says; the cut size adds what joints add | `app/features::joint_kinds_are_evaluated_per_pair`, `app/features::extend_follows_the_contact_and_fit_comes_from_joints` | covered |
| `fit` on a joint's first part: the clear distance to the parts it is joined into | `app/features::joint_kinds_are_evaluated_per_pair`, `app/features::extend_follows_the_contact_and_fit_comes_from_joints` | covered |
| With `against`/`flush`, one end is known and the size reaches the nearest joined part beyond it | `app/spec_placement::fit_against_a_face_reaches_to_the_nearest_joined_part_beyond_it` | covered |
| Joined parts that straddle the centre are ignored | `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| A side with no joined part is `fit-undecided`, saying which side | `app/features::fit_without_a_joined_part_on_one_side_is_undecided`, `app/spec_placement::fit_against_says_which_side_has_no_joined_part`, `broken/fit_undecided` | covered |
| A spaced part's size must be written | `app/spec_placement::a_spaced_parts_size_must_be_written` | covered |
| A size decided by a joint with its own error isn't also undecided | `app/one_mistake::a_size_decided_by_a_joint_with_its_own_error_is_not_also_undecided` | covered |
| `fit` on a turned part's length is the profile's last station | `app/spec_declarations::a_turned_part_is_as_long_as_its_profile` | covered |
| A turned part's box is as wide as its largest diameter | `app/spec_declarations::a_turned_parts_box_is_as_wide_as_its_largest_diameter` | covered |
| Measure a size with `between(a, b)` | `app/spec_placement::between_measures_the_clear_distance`, `app/features::between_parts_that_overlap_says_so` | covered |

### Regions

| Rule | Tests | Status |
|---|---|---|
| `opening(...)` needs all six anchors, each on its own axis | `app/spec_placement::opening_needs_all_six_sides_on_their_own_axes` | covered |
| A region is always one region expression | `app/spec_placement::a_region_is_always_a_region_expression` | covered |
| `split(region, axis, at: part)` makes two regions ending at the part's faces | `app/features::split_at_a_part_ends_at_its_faces`, `app/spec_placement::split_at_a_part_makes_two_regions` | covered |
| `split(region, axis)` makes as many equal regions as the declaration has elements | `app/features::regions_split_and_cover_with_meeting_edges`, `app/spec_placement::split_without_at_needs_an_array_to_hold_its_pieces` | covered |
| `split` labels pieces along its axis (lowest, leftmost, front first) | `app/spec_placement::split_labels_its_pieces_along_the_axis` | covered |
| `cover(region, mount, thickness:)` | `app/features::regions_split_and_cover_with_meeting_edges`, `app/spec_placement::a_mount_is_overlay_or_inset` | covered |
| `overlay(...)`: in front of the opening, each edge defaulting to 0mm, negative a reveal | `app/spec_placement::an_overlay_front_lies_in_front_of_the_opening` | covered |
| `inset(reveal:)`: inside the opening, front flush, even gap | `app/spec_placement::an_inset_front_is_flush_with_the_openings_front_with_an_even_gap` | covered |
| `meeting:` applies only between pieces of one split with nothing between them | `app/features::regions_split_and_cover_with_meeting_edges`, `app/spec_placement::meeting_applies_only_between_pieces_of_a_split_with_nothing_between_them` | covered |

### Joints and roles

| Rule | Tests | Status |
|---|---|---|
| A joint names its kind, its parts by role and its settings in one call | `app/spec_joints::the_positional_argument_binds_the_first_role` | covered |
| An unknown argument is `unknown-field`; a missing role or required param is `missing-field` | `app/features::a_misspelt_field_is_one_error_not_two`, `app/spec_joints::a_role_that_is_not_given_is_a_missing_field`, `app/spec_joints::a_kind_param_without_a_default_must_be_given` | covered |
| The positional argument binds the first role; at most one is positional | `app/spec_joints::the_positional_argument_binds_the_first_role`, `app/spec_joints::a_kind_call_takes_at_most_one_positional_argument` | covered |
| A role or setting given twice is `duplicate-name` | `app/spec_joints::a_role_or_setting_given_twice_is_a_duplicate_name` | covered |
| A joint needs a joint kind; hardware a slide, hinge or pull kind | `app/spec_joints::a_joint_needs_a_joint_kind_and_hardware_a_hardware_kind`, `app/features::hardware_named_after_its_kind_is_explained` | covered |
| A kind value carries settings; the call adds roles; its settings are readable | `app/spec_joints::a_kind_value_supplies_settings_and_the_call_adds_roles`, `app/features::kind_values_carry_settings_and_are_readable` | covered |
| A role is `Part` or `Parts` | `app/spec_joints::a_part_role_takes_one_part_and_a_parts_role_one_or_a_list`, `app/spec_joints::a_kind_with_a_parts_role_has_exactly_two_roles` | covered |
| A kind with a `Parts` role has exactly two roles and applies to the pairs that meet | `app/spec_joints::a_kind_with_a_parts_role_has_exactly_two_roles`, `app/spec_joints::a_joint_applies_to_the_pairs_that_meet` | covered |
| A kind with three or more `Part` roles is one connection | `app/spec_joints::a_kind_with_three_part_roles_is_one_connection` | covered |
| Parts meet when their boxes touch over an area or overlap (butt, cross) | `model/value::boxes_meet_when_they_touch_over_an_area`, `app/features::extend_follows_the_contact_and_fit_comes_from_joints`, `app/spec_joints::extend_on_parts_that_cross_is_a_type_mismatch` | covered |
| Every listed part must meet a counterpart (`no-contact`) | `app/spec_joints::a_joint_applies_to_the_pairs_that_meet`, `broken/joint_does_not_meet`, `broken/joint_parts_apart`, `app/fit::a_joint_whose_parts_never_touch` | covered |
| Everything in a kind is evaluated per pair | `app/features::joint_kinds_are_evaluated_per_pair`, `app/spec_joints::a_kind_require_is_checked_per_pair_and_reported_at_the_joint` | covered |
| A failing kind `require` is an error on the joint's line | `app/features::a_failing_kind_require_is_reported_at_the_joint`, `app/spec_joints::a_kind_require_is_checked_per_pair_and_reported_at_the_joint`, `broken/require` | covered |

### Joint definitions

| Rule | Tests | Status |
|---|---|---|
| Joint kinds are written in `.stave`, the standard ones too | `app/pipeline::every_example_and_standard_module_checks_clean` | covered |
| `roles { }` defaults to `piece: Parts, into: Parts` | `app/spec_joints::roles_default_to_piece_and_into`, `schema/kinds::joint_defaults_match_the_grammar` | covered |
| `adds: extend(role, by: d)` at each end where the pair butts, toward the counterpart | `app/features::extend_follows_the_contact_and_fit_comes_from_joints`, `app/spec_joints::adds_takes_a_list_of_extends` | covered |
| `adds` is `extend(role, by: length)` | `app/spec_joints::adds_is_extend_of_a_role_by_a_length` | covered |
| `extend` on a pair that crosses is `type-mismatch` | `app/spec_joints::extend_on_parts_that_cross_is_a_type_mismatch` | covered |
| Extensions of two joints at the same end add up | `app/spec_joints::extensions_of_two_joints_at_the_same_end_add_up` | covered |
| A turned part is extended at its end diameter | `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| `uses:` a fastener kind value with a count per pair (default 1); never placed | `app/spec_joints::uses_lists_fasteners_per_pair_with_a_count` | covered |
| `cut_first:` a role or a list of roles; made parts last | `app/spec_joints::cut_first_orders_the_roles_of_each_pair` | covered |
| `makes part`: once per pair, named `<joint>.<name>`, placed relative to the joint's parts | `app/spec_joints::a_made_part_is_named_after_its_joint_and_made_once_per_pair` | covered |
| The made part's place terms (`orient ... of:`, `center(selector)`, `against`/`flush` on a selector) | `app/spec_joints::a_made_part_is_named_after_its_joint_and_made_once_per_pair`, `schema/placement::made_part_terms_and_steps_match_the_grammar` | partial: `orient ... of:` and `center(selector)` are asserted; the other terms only resolve |
| Parts of a joint fit exactly as drawn after every cut (touch, never pass through) | `broken/tenon_too_long`, `broken/shelf_misses_dado`, `app/fit::every_tenon_and_shelf_of_the_examples_sits_in_its_housing`, `geom/collide::within_the_tolerance_is_touching`, `geom/collide::a_rod_in_its_hole_fits_and_a_longer_one_does_not` | covered |
| A cut the geometry can't make yet is `warning[not-supported]` | `broken/dovetail_not_cut`, `app/fit::a_dovetail_is_drawn_uncut_and_left_out_of_the_fit_check` | covered |
| Face selectors `end`, `side` and `crossing` | `app/spec_joints::a_cut_selector_is_end_side_or_crossing`, `app/spec_joints::operations_record_their_measurements`, `schema/operations::selectors_and_face_removal_match_the_grammar` | covered |
| `side` names the face across the width or thickness; `opposite:`, `flip:` | `app/spec_joints::side_names_the_face_across_the_width_or_thickness`, `app/spec_joints::operations_record_their_measurements` | covered |
| Reference face and edge default to the ones facing up, right or front | `app/pipeline::the_clothing_cabinet_is_where_the_design_says`, `app/spec_declarations::face_side_and_face_edge_are_face_words` | covered |
| Each operation is cut on its own selectors | `app/spec_joints::each_operation_is_cut_on_its_own_selectors`, `schema/operations::cut_operations_match_the_grammar` | covered |
| Operation arguments: required ones, counts, `through` | `app/spec_joints::a_required_operation_argument_is_a_missing_field`, `app/spec_joints::a_count_argument_is_a_count`, `app/spec_joints::through_cuts_all_the_way` | covered |
| The operations and their measurements | `app/spec_joints::operations_record_their_measurements`, `app/spec_joints::an_unknown_operation_lists_the_operations`, `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| `insert:` steps move a part along or about its own axes | `app/spec_joints::insert_steps_move_a_part_along_its_own_axes`, `app/spec_joints::insert_steps_name_part_axes_and_a_fixed_vocabulary` | covered |
| Steps are checked in reverse, swept continuously (`order`) | `broken/tenon_wrong_way`, `broken/chidori_no_flats`, `app/fit::a_tenon_pushed_in_the_wrong_way`, `app/fit::the_chidori_comes_apart_as_the_simulation_proves`, `app/oracle_chidori::the_lock_turns_a_quarter_and_slides_out` | covered |
| `note:` advice, with interpolation | `app/spec_joints::a_kind_note_fills_in_its_roles` | covered |

### Hardware

| Rule | Tests | Status |
|---|---|---|
| A slide checks the gap to its opening (`slide-gap`) | `broken/drawer_does_not_fit`, `app/fit::a_drawer_too_wide_for_its_slides` | covered |
| A slide gives its assembly a travel toward the front | `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| A cup hinge bores its cup; it moves its door on the built-in four-bar linkage, exact at rest | `app/spec_joints::a_cup_hinge_bores_its_cup_in_the_mount_face`, `app/pipeline::the_clothing_cabinet_is_where_the_design_says`, `app/pipeline::the_cabinet_doors_move_on_their_hinges_linkage`, `model/linkage::a_linkage_is_checked_over_its_whole_range`, `geom/linkage::the_cup_hinge_linkage_rests_exactly_and_opens_to_its_angle`, `app/oracle_hinge::closed_is_exact`, `app/oracle_hinge::the_links_reach_past_110_degrees_with_a_millimetre_to_spare`, `app/oracle_hinge::the_hinge_edge_swings_out_as_the_catalogue_says`, `app/oracle_hinge::the_door_stands_8_6mm_into_the_opening_at_110_degrees`, `app/oracle_hinge::the_front_edge_moves_forward_and_out` | covered |
| A cup hinge's overlay is `base_overlay + cup_inset - plate_height`, give or take `overlay_adjustment` (`hinge-overlay`) | `broken/hinge_overlay`, `app/fit::a_door_too_far_out_for_its_cup_hinges`, `app/oracle_hinge::the_overlay_is_the_catalogues_for_every_boring_distance` | covered |
| A cup hinge's arm, knuckle and links never put steel in wood (the door's, less only the cup's bore, or the side's) at any angle, by the swing check's sweep and by measurement; a member in wood when closed is caught at 0 | `app/focus::no_cup_hinge_puts_steel_in_wood`, `app/oracle_hinge::nothing_passes_through_wood_at_any_side_screw_setting` | covered |
| A cup hinge opens at most 110deg | `app/fit::a_cup_hinge_opens_at_most_110_degrees` | covered |
| A butt hinge cuts two leaf mortises and turns about its pin edge | `app/features::butt_hinge_leaves_are_housed_in_both_parts_at_the_flush_edge` | covered |
| A hinge, slide or pull kind names its members with `member` and a doc comment; the words are fixed per base; a misnamed member is `unknown-field`, a repeated one `duplicate-name` | `lang/parser::contextual_words_in_their_positions`, `app/spec_declarations::hardware_members_are_named_and_documented`, `app/spec_declarations::the_standard_hinges_and_slides_document_their_members`, `app/spec_declarations::a_misnamed_member_is_an_unknown_field`, `app/spec_declarations::a_fastener_has_no_members_and_a_member_is_written_once`, `broken/misnamed_member` | covered |
| A pull bores one screw hole, or two `centres` apart | `app/spec_joints::a_pull_bores_one_hole_or_two_holes_centres_apart` | covered |
| Fasteners are only used through `uses:`; tools are listed in `tools:` | `app/spec_joints::a_joint_needs_a_joint_kind_and_hardware_a_hardware_kind`, `app/spec_joints::uses_lists_fasteners_per_pair_with_a_count` | covered |
| Envelopes are a box or a cylinder | `app/spec_joints::an_envelope_is_a_box_or_a_cylinder`, `app/view::a_knob_stands_off_its_face` | covered |
| An envelope overlapping anything but its own parts is `overlap` | `broken/knob_hits_top`, `app/fit::a_knob_in_the_way_of_another_part`, `app/fit::the_examples_check_clean_with_their_cuts_made` | covered |
| Models are for looks only; a missing model is drawn as its envelope | `app/spec_joints::a_model_is_for_looks_only` | covered |
| `align: bounds` mismatch over 0.5mm is `model-mismatch` | — | pending: reading glTF models is a later step |

### Work cards and recipes

| Rule | Tests | Status |
|---|---|---|
| A recipe is for one of the eleven operations | `app/spec_joints::a_recipe_is_for_one_of_the_eleven_operations`, `lang/lib::operations_match_the_grammar` | covered |
| `tools:` lists are tool kinds; a misspelt tool is `unknown-name` | `app/spec_joints::recipe_fields_are_typed`, `app/spec_declarations::a_misspelt_tool_is_an_unknown_name` | covered |
| A library declares its own tools | `app/spec_joints::a_tool_kind_has_a_title_and_powered` | covered |
| Features and measurements belong to the operation and never share a name | `app/spec_joints::a_steps_feature_and_measurement_belong_to_the_operation`, `lang/lib::features_and_measurements_never_share_a_name`, `schema/operations::features_and_measurements_match_the_grammar` | covered |
| Marking steps in `mark { }`, cutting steps in `cut { }` | `app/spec_joints::marking_steps_go_in_mark_and_cutting_steps_in_cut`, `schema/recipes::marking_steps_match_the_grammar`, `schema/recipes::cutting_steps_match_the_grammar` | covered |
| Every step accepts `with:` (one of the recipe's tools) and `note:` | `app/spec_joints::with_names_one_of_the_recipes_tools`, `app/spec_joints::a_recipe_uses_every_step_feature_and_need_of_its_operation`, `app/one_mistake::a_misspelt_recipe_tools_field_is_one_error` | covered |
| Cutting steps accept `to:`, `passes:`, `guide:`, and `diameter:` for `bore` only | `app/spec_joints::diameter_is_for_bore_only`, `app/spec_joints::recipe_fields_are_typed` | covered |
| `needs:` flat_face, square_stock, before(op), after(op) | `app/spec_joints::recipe_fields_are_typed`, `schema/recipes::needs_match_the_grammar` | covered |
| Choosing a recipe (candidates, tools, methods, rank, ties) | — | pending: recipe choice is stave-workplan, a later step |
| A cut with no usable recipe is `warning[no-recipe]` | — | pending: recipe choice is stave-workplan, a later step |

### Order of operations

| Rule | Tests | Status |
|---|---|---|
| Phases, the ordering rules, least effort, explained steps | — | pending: the work plan is a later step (stave-workplan) |
| No valid order is `order` | `broken/no_build_order` | pending: buildability is layer 7 |

### Built-in functions

| Rule | Tests | Status |
|---|---|---|
| The built-in functions are the grammar's list | `lang/lib::builtin_functions_match_the_grammar`, `schema/functions::builtins_match_the_grammar_and_the_parser` | covered |
| `between(a, b)`: parts apart along exactly one axis, or parallel anchors | `app/spec_placement::between_measures_the_clear_distance`, `app/features::between_parts_that_overlap_says_so` | covered |
| `extent(items, axis)`, and with `of:` along another part's axis | `app/spec_placement::extent_is_the_overall_drawn_size_of_a_list_along_an_axis`, `app/features::extent_along_another_parts_axis` | covered |
| `mid(item, axis)` | `app/features::center_on_an_item_and_between_planes`, `app/spec_declarations::scene_fields_show_hide_explode_and_section` | covered |
| `opening`, `split`, `cover`, `overlay`, `inset` | see [Regions](#regions) | covered |
| `turned(profile)` | `app/spec_declarations::a_turned_part_is_as_long_as_its_profile` | covered |
| `box(...)`, `cylinder(...)`, `model(...)` | `app/spec_joints::an_envelope_is_a_box_or_a_cylinder`, `app/spec_joints::a_model_is_for_looks_only` | covered |
| `ends(inset:)`, `pitch(d)`, `gap(d)` | `app/features::spaced_pitch_and_gap`, `schema/placement::spacing_values_match_the_grammar` | covered |

### Scenes

| Rule | Tests | Status |
|---|---|---|
| Every project has the scene `rest` first; declaring `rest` is `duplicate-name` | `app/spec_declarations::every_project_has_the_rest_scene_first`, `app/spec_declarations::declaring_a_scene_called_rest_is_a_duplicate_name` | covered |
| Motions belong to assemblies; the whole assembly moves | `app/pipeline::the_clothing_cabinet_is_where_the_design_says`, `scene/cabinet::the_open_scene_swings_doors_and_slides_drawers` | covered |
| `pose` names an assembly: open, closed, an angle or a length | `app/spec_declarations::poses_are_open_closed_an_angle_or_a_length`, `app/spec_declarations::only_an_assembly_can_be_posed` | covered |
| A mirrored assembly has the mirror image of its source's motion | `app/pipeline::the_clothing_cabinet_is_where_the_design_says` | covered |
| Hinges of an assembly share one pivot, slides one direction (`motion`) | — | pending: motion checks are layer 5 |
| Every scene is collision checked, and every motion closed and fully open | `broken/scene_pose_clash`, `broken/door_hits_side`, `broken/doors_hit_each_other`, `broken/knob_hits_door`, `broken/drawer_hits_divider`, `broken/lid_hits_back`, `app/fit::every_motion_of_the_examples_is_clear_through_its_travel`, `geom/sweep::a_door_first_hits_a_post_at_the_angle_it_reaches_it`, `geom/sweep::a_drawer_hits_what_is_in_front_of_it` | covered |
| `explode: true` draws parts moved out along the reverse of their joint's last insert step; never collision checked | `app/explode::an_exploded_side_table_pulls_the_legs_off_the_aprons`, `app/explode::hardware_explodes_with_its_part_and_scenes_say_when_they_explode` | covered |

### Focus mode (the viewer)

Not language rules, but the behaviour `docs/plan.md` asks focus mode for,
pinned down the same way.

| Rule | Tests | Status |
|---|---|---|
| Holes in round and turned parts are drawn (the stool's sockets) and cut from the solid; drilled meshes, turned or board, are watertight (every edge shared by two triangles) | `app/focus::a_turned_leg_is_drawn_with_its_sockets_bored`, `app/focus::drilled_meshes_are_watertight` | covered |
| Faces a cut made are marked for tinting; the stock's own surface is not | `app/focus::cut_faces_are_marked_and_the_stock_surface_is_not` | covered |
| A joint's insert steps are traced forwards from the same run the order check makes | `app/focus::a_joint_traces_its_steps_forwards` | covered |
| Joint focus: the parts, cuts, fit and insert steps in words; the steps played from apart to together; the receiving part see-through, the rest ghosts; the home outlined | `app/focus::focus_on_a_socket_plays_the_rung_in`, `app/focus::focus_on_the_chidori_turns_the_lock_last`, `gtk/screenshot::focus_renders_offscreen` | covered |
| Focus finds joints, pairs and hardware by name, and offers a selection's joints | `app/focus::focus_finds_joints_pairs_and_hardware_by_name` | covered |
| Hardware focus: the kind's doc comment and numbered members, how far it opens, the swing check, the whole path of the door's edge (travelled solid, ahead dotted, ends marked) | `app/focus::focus_on_a_cup_hinge_names_its_members_and_measures_its_swing`, `app/focus::focus_on_a_slide_follows_the_drawer`, `app/focus::focus_on_a_knob_names_the_knob_and_its_screw` | covered |
| The smallest gap over a motion: exact for a slide, within 0.05mm for a swing; the closest approach written with where it happens, its dimension drawn only there, its ends exactly the gap apart, for slides and swings | `geom/clearance::a_slide_past_a_box_is_exact`, `geom/clearance::a_swing_finds_the_closest_corner_to_within_the_bound`, `geom/clearance::box_distance_is_exact`, `app/focus::the_smallest_gap_over_a_swing_is_within_the_bound` | covered |
| A flat view from above: cut just over a hinge's arm, through a slide's middle (turned to its travel), or through a joint whose part turns upright (the chidori) | `scene/camera::a_plan_looks_straight_down_flat_with_the_front_at_the_bottom`, `app/focus::focus_on_a_cup_hinge_names_its_members_and_measures_its_swing`, `app/focus::focus_on_a_slide_follows_the_drawer`, `app/focus::the_nightstands_slide_shows_its_drawer_travelling_from_above`, `app/focus::focus_on_the_chidori_turns_the_lock_last`, `gtk/screenshot::focus_renders_offscreen` | covered |
| Hardware focus frames the whole motion in 3D: its nodes, path and gap mark stay in view at every point | `app/focus::hardware_focus_frames_the_whole_motion` | covered |
| Overlay lines keep their dash pattern across joined pieces | `scene/focus::a_broken_line_keeps_its_pattern_across_joined_pieces` | covered |

### Checks and require

| Rule | Tests | Status |
|---|---|---|
| A require is checked at the current values | `app/spec_declarations::a_failing_require_is_reported_on_its_line_with_its_message`, `broken/require` | covered |
| A require must hold over the whole range of every param it depends on | `broken/require_range` | pending: needs z3 (layer 6) |
| Checking runs in layers; a later layer skips items with earlier errors | `app/features::a_name_error_poisons_only_its_item`, `app/features::a_syntax_error_leaves_out_only_its_item` | covered |

### Diagnostics

| Rule | Tests | Status |
|---|---|---|
| Compiler-style rendering: code, message, file:line:column, notes, help | `lang/diagnostics::renders_compiler_style`, `lang/diagnostics::syntax_error`, `lang/diagnostics::several_errors_in_one_file` | covered |
| The codes are fixed and match the table | `lang/diagnostics::codes_match_the_grammar_table` | covered |
| Every diagnostic has a concrete help line | `app/pipeline::broken_designs_produce_exactly_their_expected_diagnostic`, `app/fuzz::mutated_projects_never_panic_or_hang_and_report_well_formed_diagnostics`, `lang/fuzz::mutated_sources_never_panic_hang_or_lose_text`, `lang/spec_syntax::every_syntax_error_has_a_help_line` | covered |
| Lengths in messages in project units, trailing zeros trimmed | `lang/units::display_trims_zeros`, `app/spec_values::a_require_message_fills_in_values_in_the_projects_units` | covered |
| A syntax message's `<token>`: the text in quotes, end of line, end of file, a string, a doc comment | `lang/parser::one_mistake_one_error`, `app/spec_declarations::a_require_needs_its_message` | covered |
| The broken-design corpus: exact code and message, `// pending:` for checks not built | `app/pipeline::broken_designs_produce_exactly_their_expected_diagnostic`, `lang/broken::broken_designs_produce_their_expected_diagnostic`, `lang/lib::the_broken_corpus_expects_documented_diagnostics` | covered |
| Diagnostics are sorted and the same run to run | `app/pipeline::diagnostics_are_identical_run_to_run`, `modules/memory::diagnostics_are_sorted_and_deterministic`, `cli/determinism::every_example_gives_identical_results_run_to_run` | covered |

### Tooling and editors

| Rule | Tests | Status |
|---|---|---|
| A lossless syntax tree: printing it gives back the file | `lang/corpus::the_tree_is_lossless`, `lang/fuzz::mutated_sources_never_panic_hang_or_lose_text` | covered |
| Recovery: one mistake produces one error and the rest still checks | `app/one_mistake::one_mistake_in_any_example_is_exactly_one_error_on_its_line`, `lang/parser::one_mistake_one_error`, `lang/broken::the_rest_of_a_broken_file_still_parses` | covered |
| The parser and front end never panic or hang on any input | `lang/fuzz::mutated_sources_never_panic_hang_or_lose_text`, `lang/fuzz::pathological_inputs_finish`, `lang/parser::junk_never_hangs_the_parser`, `app/fuzz::mutated_projects_never_panic_or_hang_and_report_well_formed_diagnostics` | covered |
| Spans are byte offsets inside the file; lines and columns are computed at the edge | `lang/diagnostics::lines_and_columns_count_characters`, `lang/ast::spans_are_ordered_and_join`, `lang/fuzz::mutated_sources_never_panic_hang_or_lose_text` | covered |
| Parsing is fast enough for every keystroke | `lang/timing::the_cabinet_parses_in_well_under_a_millisecond_per_file` | covered |
| `stave fmt` layout: indentation, spacing, alignment, comments, blank lines | `lang/format::indents_and_spaces`, `lang/format::aligns_fields_in_runs`, `lang/format::aligns_consecutive_one_line_declarations`, `lang/format::keeps_comments_and_their_columns`, `lang/format::continuation_lines` | covered |
| Formatting is idempotent and keeps the tree; it refuses files with errors | `lang/corpus::formatting_is_idempotent_and_keeps_the_meaning`, `lang/fuzz::mutated_sources_never_panic_hang_or_lose_text`, `lang/format::refuses_files_with_errors`, `cli/cli::fmt_skips_files_with_errors` | covered |
| Every file of the repository is formatted | `lang/corpus::every_corpus_file_is_already_formatted`, `cli/cli::the_whole_repository_is_formatted` | covered |
| Fast and slow tiers, incremental queries, the language server and its features | — | pending: stave-lsp is a later step |

### Smart edits

| Rule | Tests | Status |
|---|---|---|
| Resize and move drags edit values through relationships; locked values stop the chain | — | pending: smart edits need z3 inverse solving, out of scope for this push (plan.md) |

### Types, units and arithmetic

| Rule | Tests | Status |
|---|---|---|
| Values are exact fractions, rounded once, to whole nanometres when stored | `app/spec_values::a_third_of_an_inch_is_exact`, `app/spec_values::the_length_unit_of_a_size_is_kept_exact_to_the_nanometre`, `lang/units::one_inch_divided_by_three_rounds_once`, `lang/units::rounding_is_half_away_from_zero` | covered |
| The cut list rounds to `resolution:` | — | pending: the cut list is a later step (stave-output) |
| length ± length, length × ratio, length ÷ ratio, length ÷ length → ratio, the same for angles, unary minus | `app/spec_values::a_length_divided_by_a_length_is_a_ratio`, `app/spec_values::a_count_may_be_used_where_a_ratio_is_expected`, `app/spec_values::unary_minus_negates_a_length`, `app/spec_values::angles_add_like_lengths`, `lang/units::exact_lengths_follow_the_arithmetic_table`, `model/resolve/expr::arithmetic_follows_the_grammar_table` | covered |
| Anything else is `unit-mismatch` | `app/spec_values::a_length_plus_an_angle_is_a_unit_mismatch`, `app/spec_values::a_count_divided_by_a_length_is_a_unit_mismatch`, `app/spec_values::an_angle_times_a_length_is_a_unit_mismatch`, `app/spec_values::a_count_minus_a_length_is_a_unit_mismatch`, `broken/unit_mismatch` | covered |
| Comparisons need the same type and give a Bool; `&&`, `||`, `!` take Bools | `app/spec_values::a_comparison_needs_the_same_type_on_both_sides`, `app/spec_values::logical_operators_take_bools` | covered |
| Dividing by zero is `type-mismatch` | `app/spec_values::dividing_by_zero_is_a_type_mismatch` | covered |
| An integer literal is a Count; a Count fits where a Ratio is expected | `app/spec_values::a_count_may_be_used_where_a_ratio_is_expected`, `model/types::counts_stand_for_ratios_and_parts_for_lists`, `model/value::counts_stay_counts_and_compare_with_ratios` | covered |
| A bare number is never a length | `app/spec_values::a_bare_number_is_never_a_length` | covered |
| The complete list of types | `lang/lib::types_match_the_grammar`, `model/types::descriptions_read_as_english` | covered |
| Closed word lists that aren't param types | `schema/words::every_word_belongs_to_its_list`, `schema/words::measures_grain_and_motion_match_the_grammar`, `model/resolve/expr::words_resolve_for_the_expected_list` | covered |
| What z3 can and can't check over a range | — | pending: z3 is layer 6 |

### Patches

| Rule | Tests | Status |
|---|---|---|
| A patch uses the same grammar; every operation parses | `lang/parser::patch_files_parse`, `lang/format::formats_patch_files` | covered |
| A patch file is told apart by its first declaration | `lang/parser::patch_files_parse`, `modules/memory::patch_file_is_not_part_of_the_module` | covered |
| Applying patches: add, replace, set, unset, remove, rename; paths; re-checking | — | pending: applying patches is a later step |

## Diagnostics

One row per code of the grammar's Diagnostics table and its warnings.

| Code | Layer | Status | Broken design | Tests |
|---|---|---|---|---|
| `syntax` | 1 | covered | `broken/syntax_error` | `lang/parser::one_mistake_one_error`, `lang/spec_syntax::every_syntax_error_has_a_help_line` |
| `missing-value` | 1 | covered | `broken/missing_value` | `lang/parser::a_missing_value_is_reported_on_its_own_line` |
| `reserved-word` | 1 | covered | `broken/reserved_word` | `lang/parser::a_keyword_where_a_new_name_goes_is_reserved`, `modules/memory::keyword_folder_without_alias_is_reserved_word` |
| `unknown-unit` | 1 | covered | `broken/unknown_unit` | `lang/lexer::quantity_forms`, `app/one_mistake::a_size_decided_by_a_joint_with_its_own_error_is_not_also_undecided` |
| `unclosed-bracket` | 1 | covered | `broken/unclosed_bracket` | `lang/parser::an_unclosed_bracket_costs_one_error`, `lang/spec_syntax::a_bracket_is_closed_where_two_values_meet_across_a_line_break` |
| `missing-brace` | 1 | covered | `broken/missing_brace` | `lang/parser::a_missing_brace_ends_the_body_at_the_next_item`, `lang/spec_syntax::a_brace_missing_at_the_end_of_the_file` |
| `unknown-name` | 2 | covered | `broken/unknown_name` | `app/features::unknown_names_and_duplicates_are_reported`, `app/spec_names::an_unknown_name_suggests_the_closest_names` |
| `duplicate-name` | 2 | covered | `broken/duplicate_name` | `app/spec_names::a_name_declared_twice_in_a_component_is_a_duplicate_name`, `app/spec_declarations::a_field_written_twice_is_a_duplicate_name` |
| `ambiguous-name` | 2 | covered | `broken/ambiguous_name` | `app/features::a_word_and_an_item_both_accepted_are_ambiguous` |
| `unit-mismatch` | 2 | covered | `broken/unit_mismatch` | `app/spec_values::a_length_plus_an_angle_is_a_unit_mismatch`, `app/spec_values::a_bare_number_is_never_a_length` |
| `type-mismatch` | 2 | covered | `broken/type_mismatch` | `app/spec_placement::against_and_flush_take_a_face_not_a_mid_plane`, `app/spec_values::logical_operators_take_bools` |
| `unknown-field` | 2 | covered | `broken/unknown_field`, `broken/misnamed_member` | `app/features::a_misspelt_field_is_one_error_not_two`, `app/spec_declarations::an_unknown_project_field_lists_the_fields`, `app/spec_declarations::a_misnamed_member_is_an_unknown_field` |
| `missing-field` | 2 | covered | `broken/missing_field` | `app/spec_declarations::a_part_needs_length_width_and_place`, `app/spec_joints::a_role_that_is_not_given_is_a_missing_field` |
| `out-of-range` | 2 | covered | `broken/out_of_range` | `app/features::values_outside_within_are_out_of_range`, `app/spec_values::a_module_params_default_must_lie_within_its_range` |
| `not-in-library` | 2 | covered | `broken/not_in_library` | `app/spec_names::every_kind_of_placed_item_is_not_in_a_library` |
| `import-cycle` | 2 | covered | `broken/import_cycle` | `modules/memory::cycle_between_two_modules`, `modules/memory::self_import_is_a_cycle` |
| `unknown-module` | 2 | covered | `broken/unknown_module` | `modules/memory::missing_folder_is_unknown_module`, `modules/memory::unknown_std_module` |
| `not-supported` | 2, 4 | covered | `broken/not_supported`, `broken/dovetail_not_cut` | `modules/memory::remote_source_is_not_supported`, `app/spec_names::a_file_in_another_format_version_is_not_supported`, `app/fit::a_dovetail_is_drawn_uncut_and_left_out_of_the_fit_check` |
| `ground` | 3 | covered | `broken/ground` | `app/features::exactly_one_part_is_ground`, `app/spec_placement::ground_is_never_inside_a_component` |
| `axis-not-set` | 3 | covered | `broken/axis_not_set` | `app/features::every_axis_is_set_exactly_once`, `app/spec_placement::the_mount_face_fixes_the_axis_it_is_normal_to` |
| `axis-set-twice` | 3 | covered | `broken/axis_set_twice` | `app/features::every_axis_is_set_exactly_once`, `app/spec_placement::a_slide_is_fixed_entirely_by_its_base` |
| `placement-cycle` | 3 | covered | `broken/placement_cycle` | `app/features::parts_placed_against_each_other_are_a_cycle`, `app/spec_placement::a_longer_placement_cycle_lists_every_part` |
| `fit-undecided` | 3 | covered | `broken/fit_undecided` | `app/features::fit_without_a_joined_part_on_one_side_is_undecided`, `app/spec_placement::fit_against_says_which_side_has_no_joined_part` |
| `mirror` | 3 | covered | `broken/mirror` | `app/features::a_mirror_without_a_counterpart_is_an_error` |
| `no-contact` | 4 | covered | `broken/butt_hinge_meets_nothing`, `broken/joint_does_not_meet`, `broken/joint_parts_apart` | `app/features::a_butt_hinge_needs_its_parts_to_meet_at_the_mount_face`, `app/fit::a_joint_whose_parts_never_touch` |
| `overlap` | 4 | covered | `broken/tenon_too_long`, `broken/shelf_misses_dado`, `broken/knob_hits_top`, `broken/pitch_runs_past_end`, `broken/door_overlay_into_top` | `app/fit::a_door_overlay_into_the_top_is_one_error_at_the_mount`, `app/fit::a_tenon_that_bottoms_out_in_its_mortise`, `app/fit::a_shelf_thicker_than_its_dado`, `app/fit::a_knob_in_the_way_of_another_part`, `app/fit::a_pitch_that_runs_past_the_end`, `geom/collide::the_depth_is_the_thinnest_side_of_what_they_share` |
| `slide-gap` | 4 | covered | `broken/drawer_does_not_fit` | `app/fit::a_drawer_too_wide_for_its_slides` |
| `hinge-overlay` | 4 | covered | `broken/hinge_overlay` | `app/fit::a_door_too_far_out_for_its_cup_hinges` |
| `motion` | 5 | covered | `broken/scene_pose_clash`, `broken/door_hits_side`, `broken/doors_hit_each_other`, `broken/knob_hits_door`, `broken/drawer_hits_divider`, `broken/lid_hits_back`, `broken/double_doors_no_gap` | `app/fit::double_doors_with_no_gap_are_told_the_gap_they_need`, `app/fit::a_lid_that_hits_the_wall_in_a_scene`, `app/fit::a_door_whose_hinge_edge_swings_into_a_wall`, `app/fit::two_doors_on_one_divider_hit_each_other`, `app/fit::a_door_that_swings_into_a_knob`, `app/fit::a_drawer_that_catches_on_a_rail`, `app/fit::a_lid_that_drops_onto_the_back` |
| `require` | 6 | covered | `broken/require` | `app/spec_declarations::a_failing_require_is_reported_on_its_line_with_its_message`, `app/features::a_failing_kind_require_is_reported_at_the_joint` |
| `require-range` | 6 | pending: checking over a param's whole range needs z3, not built yet | `broken/require_range` | — |
| `range` | 6 | pending: range checks need z3, not built yet | — | — |
| `order` | 7 | partial: reported for a joint's insert steps that can't be undone; buildability (the order of cuts, layer 7) is not built yet | `broken/tenon_wrong_way`, `broken/chidori_no_flats`, `broken/no_build_order` | `app/fit::a_tenon_pushed_in_the_wrong_way`, `app/fit::a_chidori_without_flats_cannot_come_apart` |
| `shadows-prelude` | 2 | covered | `broken/shadows_prelude` | `app/features::a_kind_hiding_the_prelude_is_a_warning`, `app/spec_names::a_recipe_hiding_a_standard_recipe_is_a_warning` |
| `no-recipe` | 7 | pending: recipe choice is stave-workplan, not built yet | — | — |
| `model-mismatch` | 4 | pending: reading glTF models is a later step | — | — |
