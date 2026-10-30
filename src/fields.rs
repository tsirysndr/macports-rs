//! Shared metadata fields with clap argument-order preservation.
use clap::{Arg, ArgAction, ArgMatches, Args, Command, FromArgMatches, parser::ValueSource};

pub const DEPENDENCIES: &[&str] = &[
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
    "depends_lib",
    "depends_run",
    "depends_test",
];
const COMMON: &[&str] = &[
    "category",
    "categories",
    "depends_fetch",
    "depends_extract",
    "depends_patch",
    "depends_build",
    "depends_lib",
    "depends_run",
    "depends_test",
    "depends",
    "description",
    "epoch",
    "homepage",
    "long_description",
    "maintainer",
    "maintainers",
    "name",
    "platform",
    "platforms",
    "portdir",
    "revision",
    "variant",
    "variants",
    "version",
];
const INFO_EXTRA: &[&str] = &[
    "conflicts",
    "fullname",
    "heading",
    "license",
    "patchfiles",
    "replaced_by",
    "subports",
];

pub fn canonical(field: &str) -> &str {
    match field {
        "category" => "categories",
        "variant" => "variants",
        "maintainer" => "maintainers",
        "platform" => "platforms",
        other => other,
    }
}

fn flags(info: bool) -> impl Iterator<Item = &'static str> {
    COMMON
        .iter()
        .copied()
        .chain(INFO_EXTRA.iter().copied().filter(move |_| info))
}
fn augment(mut command: Command, info: bool) -> Command {
    for field in flags(info) {
        command = command.arg(
            Arg::new(field)
                .long(field)
                .action(ArgAction::SetTrue)
                .help(format!("{} {field}", if info { "Show" } else { "Search" })),
        );
    }
    command
}
fn ordered(matches: &ArgMatches, info: bool) -> Vec<String> {
    let mut fields: Vec<_> = flags(info)
        .filter(|field| matches.value_source(field) == Some(ValueSource::CommandLine))
        .filter_map(|field| matches.index_of(field).map(|position| (position, field)))
        .collect();
    fields.sort_by_key(|(position, _)| *position);
    fields
        .into_iter()
        .flat_map(|(_, field)| {
            if field == "depends" {
                DEPENDENCIES.iter().map(|f| (*f).to_owned()).collect()
            } else {
                vec![field.to_owned()]
            }
        })
        .collect()
}
macro_rules! field_args {
    ($name:ident, $info:expr) => {
        #[derive(Debug, Default)]
        pub struct $name {
            pub ordered: Vec<String>,
        }
        impl Args for $name {
            fn augment_args(command: Command) -> Command {
                augment(command, $info)
            }
            fn augment_args_for_update(command: Command) -> Command {
                augment(command, $info)
            }
        }
        impl FromArgMatches for $name {
            fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
                Ok(Self {
                    ordered: ordered(matches, $info),
                })
            }
            fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
                *self = Self::from_arg_matches(matches)?;
                Ok(())
            }
        }
    };
}
field_args!(InfoFields, true);
field_args!(SearchFields, false);
