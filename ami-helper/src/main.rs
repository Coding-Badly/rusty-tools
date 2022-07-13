use std::cmp::Ordering;
use std::collections::{hash_map::HashMap, HashSet};
use std::env::{var, VarError};
use std::ops::BitOr;
use std::process::{ExitCode, Termination};

use aws_config::meta::region::RegionProviderChain;
use aws_sdk_ssm::Client;
use aws_types::region::Region;
use clap::{value_t, App, AppSettings, Arg, ArgMatches};
use futures_util::stream::StreamExt;
use once_cell::sync::Lazy;
use regex::Regex;

fn custom_error<E>(error: E) -> std::io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    std::io::Error::new(std::io::ErrorKind::Other, error)
}

pub struct UseDisplay<D>
where
    D: std::fmt::Display,
{
    exit_code: ExitCode,
    message: Option<D>,
}

impl<D> UseDisplay<D>
where
    D: std::fmt::Display,
{
    pub fn error(error: D) -> Self {
        Self {
            exit_code: ExitCode::FAILURE,
            message: Some(error),
        }
    }
    pub fn success() -> Self {
        Self {
            exit_code: ExitCode::SUCCESS,
            message: None,
        }
    }
}

impl<D> Termination for UseDisplay<D>
where
    D: std::fmt::Display,
{
    fn report(self) -> ExitCode {
        if let Some(message) = self.message {
            let text = format!("{}", message);
            eprintln!("{}", text);
        }
        self.exit_code
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperatingSystem {
    All,
    Amazon,
    Debian,
    Ubuntu,
}

impl OperatingSystem {
    fn text_width(&self) -> usize {
        <&str>::from(self).len()
    }
}

impl std::fmt::Display for OperatingSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text: &str = self.into();
        f.pad(&text)
    }
}

impl From<&OperatingSystem> for &str {
    fn from(value: &OperatingSystem) -> &'static str {
        match value {
            OperatingSystem::All => "All",
            OperatingSystem::Amazon => "Amazon Linux",
            OperatingSystem::Debian => "Debian",
            OperatingSystem::Ubuntu => "Ubuntu",
        }
    }
}

impl From<&OperatingSystem> for usize {
    fn from(value: &OperatingSystem) -> usize {
        match value {
            OperatingSystem::All => 1,
            OperatingSystem::Amazon => 2,
            OperatingSystem::Debian => 3,
            OperatingSystem::Ubuntu => 4,
        }
    }
}

impl Ord for OperatingSystem {
    fn cmp(&self, other: &Self) -> Ordering {
        let lft: usize = self.into();
        let rgt: usize = other.into();
        lft.cmp(&rgt)
    }
}

impl PartialOrd for OperatingSystem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Architecture {
    All,
    Amd64,
    Arm64,
}

impl Architecture {
    fn instance_group(&self) -> &'static str {
        match self {
            Self::All => panic!(),
            Self::Amd64 => "t3a",
            Self::Arm64 => "t4g",
        }
    }
}

impl From<Architecture> for &str {
    fn from(value: Architecture) -> &'static str {
        match value {
            Architecture::All => "all",
            Architecture::Amd64 => "amd64",
            Architecture::Arm64 => "arm64",
        }
    }
}

#[derive(Debug)]
struct SelectOptions {
    operating_system: OperatingSystem,
    architecture: Architecture,
    singleton: bool,
    just_ami: bool,
    smoke_test: bool,
    region: String,
}

impl SelectOptions {
    fn can_only_be_one(&self) -> bool {
        self.singleton || self.smoke_test
    }
    fn include_amazon(&self) -> bool {
        match self.operating_system {
            OperatingSystem::All | OperatingSystem::Amazon => true,
            _ => false,
        }
    }
    fn include_debian(&self) -> bool {
        match self.operating_system {
            OperatingSystem::All | OperatingSystem::Debian => true,
            _ => false,
        }
    }
    fn include_ubuntu(&self) -> bool {
        match self.operating_system {
            OperatingSystem::All | OperatingSystem::Ubuntu => true,
            _ => false,
        }
    }
    fn instance_group(&self) -> &'static str {
        self.architecture.instance_group()
    }
}

#[derive(Debug)]
enum AmiHelperCommand {
    Select(SelectOptions),
    Version,
}

fn build_architecture_arg<'a>() -> Arg<'a> {
    Arg::new("architecture")
        .help("Only list AMIs for the selected architecture")
        .short('a')
        .long("architecture")
        .takes_value(true)
        .multiple(false)
        .required(false)
        .value_parser(["all", "amd64", "arm64"])
}

fn build_just_ami_arg<'a>() -> Arg<'a> {
    Arg::new("just-ami")
        .help("Output just the selected AMIs")
        .short('j')
        .long("just-ami")
        .conflicts_with("smoke-test")
        .takes_value(false)
        .multiple(false)
        .required(false)
}

fn build_operating_system_arg<'a>() -> Arg<'a> {
    Arg::new("operating-system")
        .help("Only list AMIs for the selected operating system")
        .short('o')
        .long("operating-system")
        .takes_value(true)
        .multiple(false)
        .required(false)
        .value_parser(["all", "amazon", "debian", "ubuntu"])
}

fn build_region_arg<'a>() -> Arg<'a> {
    Arg::new("region")
        .help("Use this AWS region")
        .short('r')
        .long("region")
        .takes_value(true)
        .multiple(false)
        .required(false)
        .default_value("us-east-2")
}

fn build_singleton_arg<'a>() -> Arg<'a> {
    Arg::new("singleton")
        .help("Exit with an error if more than one AMI is selected")
        .short('1')
        .long("singleton")
        .takes_value(false)
        .multiple(false)
        .required(false)
}

fn build_smoke_test_arg<'a>() -> Arg<'a> {
    Arg::new("smoke-test")
        .help("Output arguments used in the smoke tests.  This argument implies --singleton.")
        .short('s')
        .long("smoke-test")
        .conflicts_with("just-ami")
        .requires("architecture")
        .takes_value(false)
        .multiple(false)
        .required(false)
}

pub fn optional<T>(input: Result<T, clap::Error>) -> Result<Option<T>, clap::Error> {
    match input {
        Ok(t) => Ok(Some(t)),
        Err(e) => match e.kind {
            clap::ErrorKind::ArgumentNotFound => Ok(None),
            _ => Err(e),
        },
    }
}

fn get_architecture_arg(matches: &ArgMatches) -> Result<Architecture, clap::Error> {
    if let Some(architecture) = optional(value_t!(matches, "architecture", String))? {
        Ok(match architecture.as_str() {
            "all" => Architecture::All,
            "amd64" => Architecture::Amd64,
            "arm64" => Architecture::Arm64,
            _ => panic!("The architecture option has a bug.  This state should be unreachable."),
        })
    } else {
        Ok(Architecture::All)
    }
}

fn get_just_ami_arg(matches: &ArgMatches) -> Result<bool, clap::Error> {
    Ok(matches.is_present("just-ami"))
}

fn get_operating_system_arg(matches: &ArgMatches) -> Result<OperatingSystem, clap::Error> {
    if let Some(operating_system) = optional(value_t!(matches, "operating-system", String))? {
        Ok(match operating_system.as_str() {
            "all" => OperatingSystem::All,
            "amazon" => OperatingSystem::Amazon,
            "debian" => OperatingSystem::Debian,
            "ubuntu" => OperatingSystem::Ubuntu,
            _ => {
                panic!("The operating-system option has a bug.  This state should be unreachable.")
            }
        })
    } else {
        Ok(OperatingSystem::All)
    }
}

fn get_region_arg(matches: &ArgMatches) -> Result<String, clap::Error> {
    value_t!(matches, "region", String)
}

fn get_singleton_arg(matches: &ArgMatches) -> Result<bool, clap::Error> {
    Ok(matches.is_present("singleton"))
}

fn get_smoke_test_arg(matches: &ArgMatches) -> Result<bool, clap::Error> {
    Ok(matches.is_present("smoke-test"))
}

mod select {
    use super::SelectOptions;
    use clap::{App, AppSettings, ArgMatches, SubCommand};

    pub(crate) const NAME: &str = "select";

    pub(crate) fn build_subcommand<'a>() -> App<'a> {
        SubCommand::with_name(NAME)
            .setting(AppSettings::NoBinaryName)
            .about("Select the AMIs that are resonable general purpose choices and match the conditions")
            .arg(super::build_architecture_arg())
            .arg(super::build_just_ami_arg())
            .arg(super::build_operating_system_arg())
            .arg(super::build_region_arg())
            .arg(super::build_singleton_arg())
            .arg(super::build_smoke_test_arg())
    }

    pub(crate) fn get_options(matches: &ArgMatches) -> Result<SelectOptions, clap::Error> {
        let operating_system = super::get_operating_system_arg(matches)?;
        let architecture = super::get_architecture_arg(matches)?;
        let just_ami = super::get_just_ami_arg(matches)?;
        let singleton = super::get_singleton_arg(matches)?;
        let smoke_test = super::get_smoke_test_arg(matches)?;
        let region = super::get_region_arg(matches)?;
        Ok(SelectOptions {
            operating_system,
            architecture,
            singleton,
            just_ami,
            smoke_test,
            region,
        })
    }
}

mod version {
    use clap::{App, AppSettings, SubCommand};

    pub(crate) const NAME: &str = "version";

    pub(crate) fn build_subcommand<'a>() -> App<'a> {
        SubCommand::with_name(NAME)
            .setting(AppSettings::NoBinaryName)
            .about("Show version information for this program")
    }
}

fn get_ami_helper_command(args: &Vec<String>) -> Result<Option<AmiHelperCommand>, clap::Error> {
    let cli = App::new("ami-helper")
        .setting(AppSettings::NoBinaryName)
        .setting(AppSettings::DisableVersion)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(select::build_subcommand())
        .subcommand(version::build_subcommand());

    match cli.get_matches_from_safe(args) {
        Ok(matches) => match matches.subcommand() {
            Some((select::NAME, options)) => Ok(Some(AmiHelperCommand::Select(
                select::get_options(options)?,
            ))),
            Some((version::NAME, _x)) => Ok(Some(AmiHelperCommand::Version)),
            _ => Ok(None),
        },
        Err(error) => Err(error),
    }
}

type BitmaskT = u128;

#[derive(Clone, Copy, Debug)]
struct StringBitmask(BitmaskT);

impl std::fmt::Display for StringBitmask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = format!("{:024b}", self.0);
        f.pad(&text)
    }
}

impl BitOr for StringBitmask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

trait StringBitmaskFilter {
    fn filter(&self, string_bitmask: &StringBitmask) -> bool;
}

struct AlwaysTrueFilter {}

impl AlwaysTrueFilter {
    fn new() -> Self {
        Self {}
    }
}

impl StringBitmaskFilter for AlwaysTrueFilter {
    fn filter(&self, _: &StringBitmask) -> bool {
        true
    }
}

struct MaskEqualsValueFilter {
    mask: StringBitmask,
    value: StringBitmask,
}

impl MaskEqualsValueFilter {
    fn new(mask: StringBitmask, value: StringBitmask) -> Self {
        Self { mask, value }
    }
}

impl StringBitmaskFilter for MaskEqualsValueFilter {
    fn filter(&self, string_bitmask: &StringBitmask) -> bool {
        (string_bitmask.0 & self.mask.0) == self.value.0
    }
}

struct OrFilter {
    filters: Vec<Box<dyn StringBitmaskFilter>>,
}

impl OrFilter {
    fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }
    fn push<F>(&mut self, filter: F)
    where
        F: StringBitmaskFilter + 'static,
    {
        self.filters.push(Box::new(filter));
    }
}

impl StringBitmaskFilter for OrFilter {
    fn filter(&self, string_bitmask: &StringBitmask) -> bool {
        if self.filters.len() > 0 {
            for filter in self.filters.iter() {
                if filter.filter(string_bitmask) {
                    return true;
                }
            }
            false
        } else {
            true
        }
    }
}

fn never_ignore(_: &str) -> bool {
    false
}

struct StringsToBitmask<'a> {
    string_to_bit: HashMap<String, u8>,
    next_bit: u8,
    combining: HashSet<String>,
    bit_to_string: Vec<String>,
    aliases: HashMap<String, HashSet<String>>,
    ignore_filter: &'a dyn Fn(&str) -> bool,
}

impl<'a> StringsToBitmask<'a> {
    pub fn new() -> Self {
        Self {
            string_to_bit: HashMap::new(),
            next_bit: 0,
            combining: HashSet::new(),
            bit_to_string: Vec::new(),
            aliases: HashMap::new(),
            ignore_filter: &never_ignore,
        }
    }
    pub fn alias<K, A>(&mut self, key: K, alias: A)
    where
        K: Into<String>,
        A: Into<String>,
    {
        let key = key.into();
        self.insert_one(&key);
        let alias = alias.into();
        self.insert_one(&alias);
        self.aliases
            .entry(key)
            .or_insert(HashSet::new())
            .insert(alias);
    }
    pub fn combining<K>(&mut self, key: K)
    where
        K: Into<String>,
    {
        self.combining.insert(key.into());
    }
    pub fn bitmask_from<'b, I>(&mut self, strings: I) -> StringBitmask
    where
        I: IntoIterator<Item = &'b str>,
    {
        let mut rv = StringsToBitmaskBuilder::new(self);
        rv.update(strings);
        rv.inner()
    }
    pub fn clear_combining(&mut self) {
        self.combining.clear();
    }
    pub fn clear_ignore(&mut self) {
        self.ignore_filter = &never_ignore;
    }
    pub fn ignore(&mut self, callme: &'a dyn Fn(&str) -> bool) {
        self.ignore_filter = callme;
    }
    pub fn insert(&mut self, key: &str) -> BitmaskT {
        let mut rv = self.insert_one(key);
        if let Some(aliases) = self.aliases.get(key) {
            for alias in aliases {
                let bit = self.string_to_bit.get(alias).unwrap();
                rv = rv | (1 << bit);
            }
        }
        rv
    }
    fn insert_one(&mut self, key: &str) -> BitmaskT {
        if (self.ignore_filter)(key) {
            0
        } else {
            let bit = if let Some(value) = self.string_to_bit.get(key) {
                *value
            } else {
                let rv = self.next_bit;
                self.next_bit += 1;
                self.string_to_bit.insert(key.to_string(), rv);
                self.bit_to_string.push(key.to_string());
                assert!(self.bit_to_string[rv as usize] == key);
                rv
            };
            1 << bit
        }
    }
}

struct StringsToBitmaskBuilder<'a, 'b, 'c> {
    strings_to_bitmask: &'a mut StringsToBitmask<'c>,
    bitmask: StringBitmask,
    contained: Option<&'b str>,
}

impl<'a, 'b, 'c> StringsToBitmaskBuilder<'a, 'b, 'c> {
    pub fn new(strings_to_bitmask: &'a mut StringsToBitmask<'c>) -> Self {
        Self {
            strings_to_bitmask,
            bitmask: StringBitmask(0),
            contained: None,
        }
    }
    fn finalize(mut self) -> StringBitmask {
        if let Some(contained) = self.contained.take() {
            self.update_bitmask(&contained);
        }
        self.bitmask
    }
    pub fn inner(self) -> StringBitmask {
        self.finalize()
    }
    pub fn update<I>(&mut self, strings: I)
    where
        I: IntoIterator<Item = &'b str>,
    {
        for rover in strings {
            self.update_one(rover);
        }
    }
    pub fn update_one(&mut self, key: &'b str) {
        if let Some(contained) = self.contained.take() {
            let combined = format!("{}-{}", contained, key);
            self.update_bitmask(&combined);
        } else {
            if self.strings_to_bitmask.combining.contains(key) {
                self.contained = Some(key);
            } else {
                self.update_bitmask(key);
            }
        }
    }
    fn update_bitmask(&mut self, key: &str) {
        self.bitmask.0 = self.bitmask.0 | self.strings_to_bitmask.insert(key);
    }
}

impl From<StringsToBitmaskBuilder<'_, '_, '_>> for StringBitmask {
    fn from(value: StringsToBitmaskBuilder<'_, '_, '_>) -> StringBitmask {
        value.finalize()
    }
}

impl From<StringsToBitmaskBuilder<'_, '_, '_>> for BitmaskT {
    fn from(value: StringsToBitmaskBuilder<'_, '_, '_>) -> BitmaskT {
        value.finalize().0
    }
}

fn common_prefix(list: &[&str], separator: char) -> String {
    match list {
        [] => "".to_string(),
        [just_one] => just_one.chars().collect(),
        _ => {
            let first = &list[0];
            let mut rightmost = usize::MAX;
            for entry in list.iter() {
                let mut match_count = 0;
                let mut last_separator = usize::MAX;
                for (lft, rgt) in first.chars().zip(entry.chars()) {
                    if match_count > rightmost {
                        break;
                    }
                    if lft != rgt {
                        if last_separator == usize::MAX {
                            if match_count < rightmost {
                                rightmost = match_count;
                            }
                        } else {
                            if last_separator < rightmost {
                                rightmost = last_separator;
                            }
                        }
                        break;
                    }
                    match_count += 1;
                    if lft == separator {
                        last_separator = match_count;
                    }
                }
            }
            if rightmost == usize::MAX {
                first.chars().collect()
            } else {
                first.chars().take(rightmost).collect()
            }
        }
    }
}

#[derive(Debug)]
struct AmiDetail {
    operating_system: OperatingSystem,
    name: String,
    ami: String,
    bitmask: StringBitmask,
}

impl Eq for AmiDetail {}

impl Ord for AmiDetail {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.operating_system.cmp(&other.operating_system) {
            Ordering::Equal => match self.name.cmp(&other.name) {
                Ordering::Equal => self.ami.cmp(&other.ami),
                o @ _ => o,
            },
            o @ _ => o,
        }
    }
}

impl PartialEq for AmiDetail {
    fn eq(&self, other: &Self) -> bool {
        self.operating_system == other.operating_system
            && self.name == other.name
            && self.ami == other.ami
    }
}

impl PartialOrd for AmiDetail {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct AmiDetailsWithFilter {
    details: Vec<AmiDetail>,
    filter: Box<dyn StringBitmaskFilter>,
}

impl AmiDetailsWithFilter {
    fn new(details: Vec<AmiDetail>, filter: Box<dyn StringBitmaskFilter>) -> Self {
        Self { details, filter }
    }
    fn into_iter(self) -> AmiDetailsWithFilterIteratorOwn {
        let details = self.details.into_iter().map(|d| Some(d)).collect();
        AmiDetailsWithFilterIteratorOwn {
            details,
            filter: self.filter,
            rover: 0,
        }
    }
}

struct AmiDetailsWithFilterIteratorOwn {
    details: Vec<Option<AmiDetail>>,
    filter: Box<dyn StringBitmaskFilter>,
    rover: usize,
}

impl Iterator for AmiDetailsWithFilterIteratorOwn {
    type Item = AmiDetail;
    fn next(&mut self) -> Option<Self::Item> {
        while self.rover < self.details.len() {
            let detail = self.details[self.rover].take().unwrap();
            self.rover += 1;
            if self.filter.filter(&detail.bitmask) {
                return Some(detail);
            }
        }
        None
    }
}

struct AmiDetailsWithFilterIteratorRef<'d> {
    target: &'d AmiDetailsWithFilter,
    rover: usize,
}

impl<'d> Iterator for AmiDetailsWithFilterIteratorRef<'d> {
    type Item = &'d AmiDetail;
    fn next(&mut self) -> Option<Self::Item> {
        while self.rover < self.target.details.len() {
            let detail = &self.target.details[self.rover];
            self.rover += 1;
            if self.target.filter.filter(&detail.bitmask) {
                return Some(detail);
            }
        }
        None
    }
}

struct NameAmiPairGetter {
    client: Client,
}

impl NameAmiPairGetter {
    async fn new(region: Region) -> Self {
        let region_provider = RegionProviderChain::first_try(region);
        let config = aws_config::from_env().region(region_provider).load().await;
        let client = Client::new(&config);

        Self { client }
    }
    async fn get_pairs(&self, path: &str) -> (Vec<String>, Vec<String>) {
        let mut response = self
            .client
            .get_parameters_by_path()
            .path(path)
            .recursive(true)
            .into_paginator()
            .send();
        let mut names = Vec::new();
        let mut amis = Vec::new();
        while let Some(chunk) = response.next().await {
            if let Ok(chunk) = chunk {
                for parameters in chunk.parameters {
                    for parameter in parameters.iter() {
                        if let (Some(name), Some(value)) = (&parameter.name, &parameter.value) {
                            names.push(name.to_string());
                            amis.push(value.to_string());
                        }
                    }
                }
            }
        }
        (names, amis)
    }
}

fn convert_pairs_to_details(
    operating_system: OperatingSystem,
    names: Vec<String>,
    amis: Vec<String>,
    all_segments: &mut StringsToBitmask,
    segment_separator: char,
) -> Vec<AmiDetail> {
    let as_str: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
    let prefix = common_prefix(&as_str, '/');
    let stripped_names: Vec<&str> = as_str
        .iter()
        .map(|n| n.strip_prefix(&prefix).unwrap())
        .collect();
    let mut details = Vec::new();
    let os_bitmask = all_segments.bitmask_from(Some((&operating_system).into()));
    for (name, ami) in stripped_names.iter().zip(amis.into_iter()) {
        let bitmask = all_segments.bitmask_from(name.split(segment_separator)) | os_bitmask;
        details.push(AmiDetail {
            operating_system,
            name: name.to_string(),
            ami,
            bitmask,
        });
    }
    details.sort();
    details
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct VersionLabel<'a> {
    version: usize,
    label: &'a str,
}

fn create_preferred_filter_for_amazon<'a, I>(
    details: I,
    all_segments: &mut StringsToBitmask,
) -> Box<dyn StringBitmaskFilter>
where
    I: IntoIterator<Item = &'a AmiDetail>,
{
    let match_version = regex::Regex::new(r"^((al|amzn)([0-9]*))-").unwrap();
    let mut versions = Vec::new();
    for detail in details.into_iter() {
        if let Some(captures) = match_version.captures(&detail.name) {
            if let (Some(label), Some(version)) = (captures.get(1), captures.get(3)) {
                let version = version.as_str();
                let version = if version == "" {
                    1
                } else {
                    version.parse::<usize>().unwrap()
                };
                versions.push(VersionLabel {
                    version,
                    label: label.as_str(),
                });
            }
        }
    }
    versions.sort();

    let mut rv = OrFilter::new();

    if versions.len() > 0 {
        let version = versions.last().unwrap();

        let mut mask = StringsToBitmaskBuilder::new(all_segments);
        mask.update_one(&version.label);
        mask.update(["kernel-default", "minimal", "amd64", "arm64"]);
        let mask = mask.inner();

        let mut value = StringsToBitmaskBuilder::new(all_segments);
        value.update_one(&version.label);
        value.update(["kernel-default", "amd64"]);
        let value = value.inner();
        rv.push(MaskEqualsValueFilter::new(mask.clone(), value));

        let mut value = StringsToBitmaskBuilder::new(all_segments);
        value.update_one(&version.label);
        value.update(["kernel-default", "arm64"]);
        let value = value.inner();
        rv.push(MaskEqualsValueFilter::new(mask.clone(), value));
    }
    Box::new(rv)
}

fn create_preferred_filter_for_debian<'a, I>(
    details: I,
    all_segments: &mut StringsToBitmask,
) -> Box<dyn StringBitmaskFilter>
where
    I: IntoIterator<Item = &'a AmiDetail>,
{
    let match_version = regex::Regex::new(r"^([1-9][0-9]*)/").unwrap();
    let mut versions = Vec::new();
    for detail in details.into_iter() {
        if let Some(captures) = match_version.captures(&detail.name) {
            if let Some(version) = captures.get(1) {
                let version = version.as_str().parse::<usize>().unwrap();
                versions.push(version);
            }
        }
    }
    versions.sort();

    let mut rv = OrFilter::new();

    if versions.len() > 0 {
        let version = versions.last().unwrap().to_string();

        let mut mask = StringsToBitmaskBuilder::new(all_segments);
        mask.update_one(&version);
        mask.update(["latest", "amd64", "arm64"]);
        let mask = mask.inner();

        let mut value = StringsToBitmaskBuilder::new(all_segments);
        value.update_one(&version);
        value.update(["latest", "amd64"]);
        let value = value.inner();
        rv.push(MaskEqualsValueFilter::new(mask.clone(), value));

        let mut value = StringsToBitmaskBuilder::new(all_segments);
        value.update_one(&version);
        value.update(["latest", "arm64"]);
        let value = value.inner();
        rv.push(MaskEqualsValueFilter::new(mask.clone(), value));
    }
    Box::new(rv)
}

fn create_preferred_filter_for_ubuntu<'a, I>(
    details: I,
    all_segments: &mut StringsToBitmask,
) -> Box<dyn StringBitmaskFilter>
where
    I: IntoIterator<Item = &'a AmiDetail>,
{
    let match_version = regex::Regex::new(r"^([1-9][0-9]*)[.]([0-9][0-9])/").unwrap();
    let mut versions = Vec::new();
    for detail in details.into_iter() {
        if let Some(captures) = match_version.captures(&detail.name) {
            if let (Some(major), Some(minor)) = (captures.get(1), captures.get(2)) {
                let major = major.as_str().parse::<usize>().unwrap();
                let minor = minor.as_str().parse::<usize>().unwrap();
                let version = major * 100 + minor;
                versions.push(version);
            }
        }
    }
    versions.sort();

    let mut rv = OrFilter::new();

    if versions.len() > 0 {
        let version = versions.last().unwrap();
        let version = format!("{}.{:02}", version / 100, version % 100);

        let mut mask = StringsToBitmaskBuilder::new(all_segments);
        mask.update_one(&version);
        mask.update(["stable", "current", "amd64", "arm64"]);
        let mask = mask.inner();

        let mut value = StringsToBitmaskBuilder::new(all_segments);
        value.update_one(&version);
        value.update(["stable", "current", "amd64"]);
        let value = value.inner();
        rv.push(MaskEqualsValueFilter::new(mask.clone(), value));

        let mut value = StringsToBitmaskBuilder::new(all_segments);
        value.update_one(&version);
        value.update(["stable", "current", "arm64"]);
        let value = value.inner();
        rv.push(MaskEqualsValueFilter::new(mask.clone(), value));
    }
    Box::new(rv)
}

struct DetailsReporter {
    os_width: usize,
    name_width: usize,
    ami_width: usize,
}

impl DetailsReporter {
    fn new() -> Self {
        Self {
            os_width: 12,
            name_width: 30,
            ami_width: 21,
        }
    }
    fn output<'a, I>(&self, details: I)
    where
        I: IntoIterator<Item = &'a AmiDetail>,
    {
        println!(
            "{0:-^1$}  {2:-^3$}  {4:-^5$}",
            " OS ", self.os_width, " Name ", self.name_width, " AMI ", self.ami_width
        );
        for rover in details.into_iter() {
            println!(
                "{0:<1$}  {2:<3$}  {4:<5$}",
                rover.operating_system,
                self.os_width,
                rover.name,
                self.name_width,
                rover.ami,
                self.ami_width
            );
        }
        println!(
            "{0:-^1$}  {2:-^3$}  {4:-^5$}",
            "", self.os_width, "", self.name_width, "", self.ami_width
        );
    }
    fn update_column_widths<'a, I>(&mut self, details: I)
    where
        I: IntoIterator<Item = &'a AmiDetail>,
    {
        let mut os_width = self.os_width;
        let mut name_width = self.name_width;
        let mut ami_width = self.ami_width;

        for detail in details.into_iter() {
            if detail.operating_system.text_width() > os_width {
                os_width = detail.operating_system.text_width();
            }
            if detail.name.len() > name_width {
                name_width = detail.name.len();
            }
            if detail.ami.len() > ami_width {
                ami_width = detail.ami.len();
            }
        }
        self.os_width = os_width;
        self.name_width = name_width;
        self.ami_width = ami_width;
    }
}

async fn do_select(options: SelectOptions) -> Result<(), Box<dyn std::error::Error>> {
    let getter = NameAmiPairGetter::new(Region::new(options.region.clone())).await;
    let mut all_segments = StringsToBitmask::new();
    all_segments.alias("x86_64", "amd64");
    let mut operating_systems: Vec<AmiDetailsWithFilter> = Vec::new();

    if options.include_amazon() {
        let (names, amis) = getter
            .get_pairs("/aws/service/ami-amazon-linux-latest")
            .await;
        all_segments.combining("kernel");
        all_segments.clear_ignore();
        let details =
            convert_pairs_to_details(OperatingSystem::Amazon, names, amis, &mut all_segments, '-');
        let preferred = create_preferred_filter_for_amazon(&details, &mut all_segments);
        let amazon = AmiDetailsWithFilter::new(details, preferred);
        operating_systems.push(amazon);
    }

    if options.include_debian() {
        let (names, amis) = getter.get_pairs("/aws/service/debian/release").await;
        all_segments.clear_combining();
        all_segments.ignore(&|s| {
            static DATE_SERIAL: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^\d{8}-\d+$").unwrap()
            });
            DATE_SERIAL.is_match(s)
        });
        let details =
            convert_pairs_to_details(OperatingSystem::Debian, names, amis, &mut all_segments, '/');
        let preferred = create_preferred_filter_for_debian(&details, &mut all_segments);
        let debian = AmiDetailsWithFilter::new(details, preferred);
        operating_systems.push(debian);
    }

    if options.include_ubuntu() {
        let (names, amis) = getter
            .get_pairs("/aws/service/canonical/ubuntu/server")
            .await;
        all_segments.clear_combining();
        all_segments.ignore(&|s| {
            static DATE_REVISION: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^\d{8}(?:[.]\d+)?$").unwrap()
            });
            DATE_REVISION.is_match(s)
        });
        let details =
            convert_pairs_to_details(OperatingSystem::Ubuntu, names, amis, &mut all_segments, '/');
        let preferred = create_preferred_filter_for_ubuntu(&details, &mut all_segments);
        let ubuntu = AmiDetailsWithFilter::new(details, preferred);
        operating_systems.push(ubuntu);
    }

    let architecture_filter: Box<dyn StringBitmaskFilter> =
        if options.architecture != Architecture::All {
            let mask = all_segments.bitmask_from(["amd64", "arm64"]);
            let value = all_segments.bitmask_from([options.architecture.into()]);
            Box::new(MaskEqualsValueFilter::new(mask, value))
        } else {
            Box::new(AlwaysTrueFilter::new())
        };
    let mut details: Vec<AmiDetail> = Vec::new();
    for section in operating_systems.into_iter() {
        for detail in section.into_iter() {
            if architecture_filter.filter(&detail.bitmask) {
                details.push(detail);
            }
        }
    }

    if options.can_only_be_one() && details.len() != 1 {
        return Err(Box::new(custom_error(format!(
            "singleton or smoke-test was specified but {} AMIs were selected",
            details.len()
        ))));
    }

    if options.smoke_test {
        print!("--image-id \"{}\" --instance-type \"{}.medium\"", details[0].ami, options.instance_group());
    } else if options.just_ami {
        if details.len() == 1 {
            print!("{}", details[0].ami);
        } else {
            for detail in details.iter() {
                println!("{}", detail.ami);
            }
        }
    } else {
        println!();
        let mut reporter = DetailsReporter::new();
        reporter.update_column_widths(details.iter());
        reporter.output(details.iter());
        println!();
    }

    Ok(())
}

async fn inner_main() -> Result<(), Box<dyn std::error::Error>> {
    let mut errors = Vec::new();
    match var("AWS_ACCESS_KEY_ID") {
        Err(VarError::NotPresent) => errors.push("AWS_ACCESS_KEY_ID is not set.  It must be set to a valid AWS access key ID."),
        Err(VarError::NotUnicode(_)) => errors.push("While AWS_ACCESS_KEY_ID is set it is not valid Unicode.  It must be set to a valid AWS access key ID."),
        Ok(_) => {}
    }
    match var("AWS_SECRET_ACCESS_KEY") {
        Err(VarError::NotPresent) => errors.push("AWS_SECRET_ACCESS_KEY is not set.  It must be set to a valid AWS access key ID."),
        Err(VarError::NotUnicode(_)) => errors.push("While AWS_SECRET_ACCESS_KEY is set it is not valid Unicode.  It must be set to a valid AWS access key ID."),
        Ok(_) => {}
    }
    if errors.len() > 0 {
        return Err(Box::new(custom_error(errors.join("  "))).into());
    }

    let raw_args = std::env::args().skip(1).collect::<Vec<String>>();
    let t = get_ami_helper_command(&raw_args);
    match t {
        Ok(Some(command)) => match command {
            AmiHelperCommand::Select(options) => do_select(options).await,
            AmiHelperCommand::Version => {
                const VERSION: &str = env!("CARGO_PKG_VERSION");
                println!("{}", VERSION);
                Ok(())
            }
        },
        Ok(None) => panic!("get_ami_helper_command has a bug.  This state should be unreachable."),
        Err(e) => {
            if e.kind == clap::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                eprintln!("{}", e);
                Ok(())
            } else {
                Err(Box::new(custom_error(e)).into())
            }
        }
    }
}

#[tokio::main]
async fn main() -> UseDisplay<Box<dyn std::error::Error>> {
    match inner_main().await {
        Ok(()) => UseDisplay::success(),
        Err(error) => UseDisplay::error(error),
    }
}
