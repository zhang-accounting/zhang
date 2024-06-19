# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.10] - 2024-06-19

### Added

- Add umami trace script (#344)
- Add span info in process error
- Add rust_log env in docker

### Changed

- We support multiple leading zero in amount
- Make fmt happy
- Bump to v0.1.10

### Fixed

- Open directive should have higher priority then balance directive
- Pad info should be removed after being used
- Use real temp dir
- Get the correct unit when processing txn

## [0.1.9] - 2024-06-14

### Added

- Add discord and testflight link in readme.md (#336)
- Add anchor link in error box to navigate to document
- Add some error codes' description

### Changed

- Resolved #265 translate all documents into english
- Upgrade opendal to support non-standard webdav server

### Fixed

- Fix the testflight icon
- Fix the testflight icon (#337)

### Removed

- Remove TransactionDoesNotBalance in locale file
- Remove useless struct

## [0.1.8] - 2024-06-04

### Added

- Add some log for cache logic
- Add custom error
- Add commodity doc
- Add breakpoint for commodity grid for better display on mobile
- Add some skeletons
- Add single account balance history graph
- Add async process method
- Support implicit posting cost
- Add validation rule for fava data
- Add default booking method options

### Changed

- Resolved #316 sort account in txn edit modal
- Resolved #314 close txn modal after submitting
- Make clippy happy
- Make prettier happy
- Return group in commodity api
- Show commodities in groups
- Show page title in html title
- Group hooks
- Use segment control for document list layout switcher
- Make prettier happy
- Upgrade to mantine 7
- Auto complete does not need creatable
- Extract router module
- New chart
- New logic of sequencedDate
- Improve accessibility
- Split once
- Move typescript pkg into dev dep
- Make prettier happy
- Update the account balance graph first commodity color to primary color
- Enhance the render logic
- Publish the docker image with tag
- Make clippy happy
- Using zhang custom error instead of unwrap the error
- Make clippy happy
- Use lightbox to preview the document image
- Only image can be preview
- Implement a new commodity lot record logic
- Handle single cost scenario
- Introduce new error kind for no enough lot record's amount
- Simplify inventory
- Handle implicit cost in posting
- Ignore unbalance txn error for txns which contains implicit cost postings
- Make clippy happy

### Fixed

- Use tokio fs instead std fs to make sure everything runs on async
- Return empty u8 vec if file does not exist
- #310 limit budget percentage to maximum 100%
- Return correct price commodity
- Fix table style
- Cols setting
- Select in account balance line
- Center of the pagination
- Notification system
- Action button color
- Typo commodities
- Size using calc
- Table with border
- Divider label position on the left
- Use absolute route
- Position of budget's progress bar
- Import dropzone style file
- Set min and max for account balance history graph
- Move test source to temp folder for cleaning testing
- Fix the case of multiple transaction date
- Fix the indent of list
- Show cost info in commodity lots page

### Removed

- Remove some console.log
- Remove mut flag in test
- Remove self-implemented github service
- Remove some useless function
- Remove expect to prevent panics
- Remove logging filter
- Remove useless method
- Remove zero amount lot record
- Remove price field in lot's record
- Remove hardcode uuid for reporting

## [0.1.8-beta.3] - 2024-04-27

### Fixed

- Fix the docker image tag

## [0.1.8-beta.2] - 2024-04-27

### Changed

- Make fmt happy
- Handle ctrl+c
- Make document cacheable

### Fixed

- Fix deprecated function

## [0.1.8-beta.1] - 2024-04-25

### Added

- Add translation for DefineDuplicatedBudget error
- Add transition for border color
- Add the mapper plugin handler
- Add router plugin related struct
- Add git sha tag for docker image
- Add plugin feature test
- Add wasm build test in PR flow
- Add plugin feature on docker build
- Add the macro tu enable feature or not
- Add plugin list in frontend setting page
- Add api of updating transaction
- Add edit transaction in frontend
- Add integration test
- Add more build target
- Add libssl in ubuntu runner
- Add pkgconfig env
- Add cross config for aarch64

### Changed

- Need to pass the value type when retrieving the option
- Extract validate method to make code readable
- Migrate update doc from github wiki to docs
- Use new upgrade link
- New UI of statistic box
- New UI of section
- Bump h2 from 0.3.24 to 0.3.26
- Plugin directive support meta.
- Use extism to implement a new plugin system and implement the processor plugin
- Make plugin as module folder
- Create cache folder if not exist
- Should not handle include directives in process method
- Make clippy happy
- Rename the feature as plugin_runtime
- Correct feature name in macro
- Restore the integration test
- Move app state to separate file
- Handle the flag for transaction
- Use From trait instead of Into
- Make clippy happy
- Combine the payee and narration
- Use once cell to detect timezone once
- Make fmt happy
- Use journal item's tags and links directly
- Show is_balanced flag in journal list and render the error
- Passing the options into plugin context
- Use trait to standard the plugin behavior
- Make clippy happy
- Download the frontend build artifact
- Upload the build artifact to github release
- Use podman to build docker image
- Notify example repo to rebuild example image
- Only trigger the CI when push to main
- Set continue on error to true
- Trigger ci when tags are pushed
- Make some dependencies into workspace

### Fixed

- Fix the feature name
- Fix github sha for docker image tag
- Fix docker image tag
- Fix the feature in dockerfile
- Fix the testcase
- Fix the testcase
- Raise unbalanced error given unbalance txn
- Fix the download artifact file

### Removed

- Remove useless import
- Remove plugin directive in integration test
- Remove the arm build
- Remove armv7 target
- Remove musl target
- Remove pkg config path
- Remove i686 target and add cross config for riscv64
- Remove riscv64 build target

## [0.1.7] - 2024-03-28

### Fixed

- Non-existed commodity wont break the process of transaction

## [0.1.6] - 2024-03-28

### Added

- Add MultipleOperatingCurrencyDetect error
- Add MultipleOperatingCurrencyDetect in frontend
- Add some translation and use pnpm
- Add pnpm.lock file

### Changed

- Install pnpm first

### Fixed

- Fix the command of publishing to npm
- Skip the process for those invalid txn
- Fix test code

### Removed

- Remove dbg macro in test code

## [0.1.5] - 2024-03-27

### Changed

- Show parse error msg as human readable format
- Zhang-ast support any str as flag
- Zhang parser and beancount parser support any upper char as the txn flag

## [0.1.4] - 2024-03-26

### Changed

- Zhang parser support comma and underline symbol for number
- Beancount parser support comma and underline symbol for number
- Make fmt and clippy happy
- Release wasm when release zhang
- Temp disable coverage

### Fixed

- Fix wasm version update script

## [0.1.3] - 2024-03-26

### Added

- Add sha in create or update github content request
- Add number expression support in beancount parser
- Add some comment for beancount number expr parsing & make clippy happy
- Add github datasource document
- Add code highlight for github datasource
- Add i18n for documentation
- Add discord link

### Changed

- Move example dockerfile into docker folder
- Move example docker image snapshot build
- Trigger example build after snapshot build is done
- You can navigate to account page in document page
- Impl github backend
- Update versions of actions, replace actions-rs/toolchain
- Run cargo fmt
- Ignore clippy::empty_docs warning explicitly
- Bump versions of multiple dependencies
- Run cargo fmt & clippy on nightly toolchain
- Use setup-node action to ensure caching
- Run coverage test on stable rust toolchain
- Make clippy happy
- #260 use starlight as document generate tool
- Migrate the arithmetic logic from beancount parser to zhang parser
- Make clippy happy
- Make fmt haapy

### Fixed

- Fix example Dockerfile
- Should quote document path to make sure we can parse files whose name contains space
- Refresh document list after document is uploaded
- Return correct file extension and render it in document page
- Show documents in datetime desc order
- Fix cargo clippy warnings
- Fix deprecation in base64 crate
- Correct inpropriate use of BigDecimal in parser test
- Fix options of setup-node action
- Http 0.2 and 1.1 dependency
- Fix logo path
- Fix link of documentation

## [0.1.2] - 2024-02-28

### Changed

- Return store in wasm
- Update zhang-wasm version to 0.1.0
- Make clippy happy
- Update readme and add playground link

### Fixed

- Handle empty space line for zhang and beancount

### Removed

- Remove Instant now access, cause it's not working in wasm
- Remove debug code

## [0.1.1] - 2024-02-26

### Added

- Add active style for navbar
- Add account filter
- Add border for account table
- Add postings into txn domain

### Changed

- Use checkbox for hide closed account
- Journal should filter by keyword now
- Account filter ignore case
- Journal list support search
- Use postings in txn directly

### Fixed

- Txn filter should ignore case

### Removed

- Remove useless import

## [0.1.0] - 2024-02-21

### Added

- Add Local file system data source

### Changed

- Introduce new DataType and DataSource, impl ZhangDataType
- Make clippy happy
- Rename variables from transformer like to data_source
- Beauty the style of new version

### Fixed

- Impl DataSource for Opendal
- Test cases
- Impl opendal data source

### Removed

- Remove exporter trait
- Remove transform.rs
- Remove database relative files
- Remove test data source

## [0.1.0-beta.6] - 2024-01-21

### Fixed

- Version file generate script

## [0.1.0-beta.5] - 2024-01-21

### Changed

- Update icon for frontend
- Load version from standard file
- Set build version in release action

### Fixed

- Pin package version to 0.1.0
- Version fo packages

### Removed

- Remove useless variable

## [0.1.0-beta.4] - 2024-01-16

### Changed

- Group journal item by day

### Fixed

- Txn's attachment path should be trimmed prefix

## [0.1.0-beta.3] - 2024-01-04

### Added

- Add document for booking method
- Add mutate in useEffect deps

### Changed

- Beancount support booking_method
- Trigger ledger reload when new transaction is added
- Reload info_for_new_transaction api when ledger is reloaded
- Report a random generated uuid

### Fixed

- #214 support trailing space at the end of line

## [0.1.0-beta.2] - 2024-01-01

### Added

- Add trx sequence
- Add payee test
- Add accounts test
- Add example file for testing
- Add DocumentType test
- Add account balance test
- Add refresh button in journal and account page
- Add python binding
- Add ledger parse methods
- Add ledger options and test python file
- Add account access method
- Add account repr for pythong binding
- Add commodity domain
- Add amount and postings domain
- Add bin name in Dockerfile
- Add doc structure
- Add doc build step
- Add logo
- Add zhang perfix for libs
- Add store api
- Add integration-tests structure
- Add integration test framework
- Add start:demo command
- Add example deployment docker file
- Add budget store and budget directive logic
- Add budget integration test files
- Add budget list api
- Add translation for budget nav
- Add progress bar in budget item
- Add budget change by txn
- Add border for journal table
- Add arrow left and right icon to jump to previous or next month
- Add key for iterable budget item to re-render component for refresh the progress bar
- Add budget info api
- Add game-expense related accounts test
- Add budget beancount parser
- Add web basic auth
- Add auth_credential for integration test
- Add multiple postings test
- Add logo
- Add reload api
- Add documentations
- Add info for enabling the frontend feature
- Add inline comment test

### Changed

- Isolate insert_commodity
- Isolate all open accounts
- Isolate all payees
- Isolate static
- Isolate all account
- Isolate transaction and price query
- Isolate lot relative query
- Format code
- Migrate options into in-memory store
- Migrate commodity into in-memory store
- Migrate lots into in-memory store
- Migrate balance into in-memory store
- Format code
- Reorder the imports
- Make clippy happy
- Move text target to a mod
- Make clippy happy
- Seperate routes into modules
- Update document
- Flatten balance check and pad
- Impl new json path test strategy
- Format code
- Make clippy happy
- Statistic api
- Code format
- Export domains
- Make fmt and clippy happy
- Bump rustix from 0.36.11 to 0.36.17
- Use pathbuf if path does not containt glob char
- Make fmt and cliipy happy
- Doc ci
- Uses to run
- Move command
- Mv command 2
- Zhang id
- Zhang id2
- Update ci permission
- Update overview
- Bump axios from 1.2.0 to 1.6.0 in /frontend
- Bump @babel/traverse from 7.17.9 to 7.23.3 in /frontend
- Update online demo uri
- Update readme.md
- Bump openssl from 0.10.55 to 0.10.60
- Update to 0.1.0-alpha.6
- Implement integration test logic
- Make clippy happy
- Move logo to assets
- Move wasm into bindings
- Move example deployment into folder
- Budget directive parser and exporter
- Budget-add directive parser and exporter
- Budget-transfer directive parser and exporter
- Budget-close directive parser and exporter
- Handle budget add directive logic
- Handle budget transfer directive logic
- Handle budget close directive logic
- Impl budget page
- Budget activity amount coming from txn
- Show search and new transaction btn in two rows
- Can select date in budget page
- Activity amount of budget should be clear when month changes
- Interval event list and api
- Make clippy happy
- Format code
- Custom directive support meta
- Make clippy happy
- Format frontend code
- Format frontend code
- Handle beancount exporter
- Make clippy happy
- Zhang's posting support meta
- Exporter for zhang posting's meta
- Beancount parser support posting meta
- New doc framework
- A working version of involving opendal
- Use beancount parser if it's beancount format
- Extract server app creation function
- Format code
- Trigger reload when file changes
- Make clippy happy
- Inline comment supported for zhang data type
- Inline comment supported for beancount data type
- Make clippy happy
- Use codemirror as editor
- Version 0.1.0-beta.2

### Fixed

- Commodity balance calculation
- Use resolver version 2 for whole workspace
- Combine same price commodity lot
- Frontend route path
- Fix clippy
- Format python test code
- Return owned string for amount currency
- Fix ci
- Show hour in 24 hour format
- Load validation points and print it.
- Fix core path for wasm
- Fix budget assigned_amount integration test
- Use string as hash key
- Should not return non-opened budget in budget list
- Should only return budget related accounts
- Integration test
- Frontend packaging
- Expose inner tuple to fix test
- Fix documentation uri
- Docker build rust version
- Create txn should use post method
- Use empty string if payee is null
- Base image for runtime

### Removed

- Remove sqlx related code and enable wasm
- Remove sqlite feature
- Remove all sqlite and sqlx query
- Remove sqlx in server lib
- Remove async in core lib
- Remove tokio dependency
- Remove useless imports
- Remove useless file
- Remove github docker registry
- Remove serde feature
- Remove useless build script
- Remove leading slash
- Remove commented tests
- Remove dbg test macro
- Remove shadow for section
- Remove useless variable
- Remove useless database parameter
- Remove comment
- Remove async dead blocking task
- Remove useless styles

## [0.1.0-alpha.5] - 2023-08-17

### Changed

- Move text tranformer and export back to core
- Centralize the transformer and exporter for beancount

### Fixed

- Resolved #175 to use hk as default timezone

## [0.1.0-alpha.4] - 2023-06-05

### Added

- Add prettier command
- Add frontend style check

### Changed

- Show non leaf account amount
- Run prettier
- Show multiple commodity in account page
- Make prettier happy
- Upgrade react to version 18 and fix the date range in report page
- Bump to 0.1.0-alpha.4

### Fixed

- Fix the prettier check command
- Show currency symbol after minus mark
- #159 remove txn not balance error for given non-balanced check directive

### Removed

- Remove pointer cursor for non leaf account

## [0.1.0-alpha.3] - 2023-05-31

### Added

- Add update command
- Add update notification

### Changed

- Bump to 0.1.0-alpha.3

### Fixed

- Typo of the exporter

### Removed

- Remove release tag prefix

## [0.1.0-alpha.2] - 2023-05-30

### Added

- Add frontend build and cache

## [0.1.0-alpha.1] - 2023-05-30

### Added

- Add idea folder to git ignore
- Add wasm lib
- Add wasm lib
- Add cli and website
- Add open and plugin parser
- Add custom directive
- Add transaction
- Add comment
- Add transaction tags and links
- Add readme
- Add account and amount
- Add draw of beancount implement
- Add forbid unknown payee configu
- Add beancount exporter
- Add date or datetime info into ast
- Add meta output for text target.
- Add chinese name
- Support date hour
- Add complex sort test
- Add docs.rs badge
- Add development and pr action
- Add development and pr action
- Add balance directive logic
- Add silent init flag
- Add graphql entry for backend server
- Add basic journal page
- Add journal page of frontend
- Add statistic box component.
- Add file listener
- Add statistic dto
- Add append data mutation
- Add empty line between transaction content
- Support multiple postings add append, deletion action
- Add git pre-commit hook
- Add block for section
- Add account check ui
- Add meta query for account
- Add price grip using latest map
- Add price grip for account snapshot
- Add distance for statistic
- Add checkbok for hide closed account
- Add eslint for frontend
- Add test
- Add price test
- Add test
- Add multiple value map test
- Add latest map test
- Add price grip test
- Add account test
- Add amount test
- Add balance pad parse test
- Add inventory test
- Add idx key for loop render
- Add snpshot build action file
- Add snapshot build name
- Add non begin at zero for income and expense axis
- Add cli hint msg
- Add statistic journal list
- Add journal to statistic
- Add status bar in report page
- Add accountType and sign for account
- Add docker file and build as snapshot
- Add run-on to fix snapshot build
- Add journal side preview content for journal page
- Add tags and links resolver
- Add timestamp for transaction
- Add account does not exist error type
- Add modal to show and edit ledger error
- Add filename to span info
- Add error length query for error box title
- Add account closed error type
- Add commodity does not define error
- Add default tolerance precision
- Add default tolerance precision
- Add commodityInventory for single commodity
- Add inventory for commodities
- Add local proxy to suit github codespaces development
- Add single commodity page with lot detail
- Add price history for single commodity
- Add sample i18n code
- Add latest balance time for single account
- Add cargo lock and config for notify
- Add toolchain setting for test job
- Add default commodity info for operating currency
- Add new ui for document line
- Add wechat extractor into tool list
- Add precision col in commodities table
- Add meta modification part
- Add pading on meta line in transaction preview page
- Add close badge for closed account
- Add empty line on beginning of directive
- Add table view for documents
- Add sqlite support MVP
- Add file path in error
- Support serve to specify database location
- Add meta for transactions
- Add database log
- Add span info into transaction table
- Add commodity info for default currency
- Add doucment icon for those have document meta
- Add frontend analyze tool
- Add backend server base url into file update api
- Add balance check record
- Add commodity state
- Add unlimited idle time for memory database
- Add unlimited idle time for memory database
- Add title in info api
- Add loader page
- Add dependencies for fmt and clippy
- Add example docker image build process
- Add readonly for example docker
- Add readonly for example docker
- Add readonly for example docker
- Add simple balance check preview
- Add notification after transaction is created
- Add husky hook
- Add responsive view for setting
- Add missing dependencies
- Add debug trait for error kind
- Add balance check for account
- Add balance pad to account
- Add masked amount for balance page
- Add backend api and reflect on unbalance amount logic
- Add beancount parser
- Add type annotation for tag stack
- Support file with bean extension
- Add test for open directive without commodity
- Add dot env file to git ignore
- Add commodity test
- Add report graph component
- Add account status in account balance table
- Add test for meta
- Add options api and show them as table in setting page
- Add test for geting all options
- Add default option override test
- Add variant of closing non zero account
- Add frontend i18n for error
- Add test for closing account
- Add release yml

### Changed

- Init
- Typo license
- Merge from beancount
- Replace beancount to avaro
- Char instead of string
- Pest parser
- Implement a lot directive
- Implement commodity
- Open directive support meta
- Implement comment
- Update readme.md
- Structure posting meta
- Implement transaction
- Foramt
- Project structure
- #[allow(clippy::upper_case_acronyms)]
- Make cargo fmt and clippy happy
- #2 open directive support meta
- Implement wechat importer
- Resolve #5 date time now support datetime
- Impl text target
- Export meta data
- Convert datetime back to date and time in meta.
- Ledger struct
- Impl text target for all directives
- Separate quote string and unquote string
- Optional config to output result to file
- Resolve #6 lower case rule
- Wechat export unknown payee
- #10 rename to zhang
- Update project badge
- Tasking
- Structure option, plugin, include and comment
- Typo
- Sort directive on loading
- Extract open directive info
- Format badge
- Close directive
- Commodity directive
- Update coveralls badge
- #9 load single file
- Resolve #9 multiple file support
- #11 add snapshot struct
- #11 calculate normal posting data.
- #11 calculate unit given one missing unit posting
- #11 calculate price based posting
- #11 calculate single price based posting
- #11 add daily account snapshot
- #3 remove pad directive
- Resolve #3 balance directive with pad
- Embedded server and frontend for ui
- Update readme.md
- Build frontend after backend build
- Return account snapshot api
- Transaction and balance check dto, and cors layer
- Badge for journal item
- Account page.
- Store entry for reload purpose.
- Reload logic
- Cargo fmt
- Cargo fmt
- Raw editing for files in frontend ui
- Get nearest day snapshot
- Statistic logic
- Single account journal
- New transaction modal data.
- Update git ignore
- Update git ignore
- Date time picker and date only checkbox
- Load account list in modal
- Save button validator
- Refetch relative query once transaction is add
- Load documents from document directive
- Document frontend
- Account document line with extension badge
- Navigate to account in document page.
- Simple error dto
- Update hook
- Format code
- Update axum extension
- Implement account balance check api
- Implement account document
- Implement account document frontend ui
- Account line show currency snapshot summary
- Update statistic to generic from to end timestamp for more generic
- Load option directive
- Extract process logic to Trait Directive Process
- Multiple currency support
- Statistic using from and to
- Customize category instead of hard code category
- Using customize group for statistic bar
- Snapshot rename to inventory
- Record snapshot in detail
- Process price directive
- Reorder liabilities statistic box
- Frontend type
- Separate frontend-build
- Test the doctest as well
- Use nightly toolchain
- Save code coverage as artifact
- Move inventory to separate mod
- Update readme
- Using create-react-app instead of customize parcel builder
- Update pipeline
- Use JournalLine to hide the Journal detail
- Allow statistic to separate into frames by day
- MultiAxis chart implementation
- Homepage statistic
- Update example accounting
- Implement report page
- Report setting
- Report page ui enhance
- Serve a path
- Login ghcr.io
- Rename artefact
- Transaction preview
- Transaction meta support
- Resolve #25 meta support same name key
- Show meta block only when metas is not empty
- Center new transaction button
- #52 publish image to docker hub
- Upload account documents
- #53 parsing before append it into files
- Select component style
- #37 using selectable for payee input
- #49 show transaction document in document list
- #49 hide document meta in meta list
- #49 show document in transaction preview page
- #49 format code to make clippy happy
- Resolve #8 add span info in directive to support raw editing
- Show error msg in graphql
- Show error summary in log
- Error pagination and list rendering
- Format code
- Format code
- Modal of updating directive with error
- Reload ledger and reload error list after file modification
- Check transaction balanced
- Show unbalanced info in journal list and preview page
- Parse cost and price in to posting
- Implement trade_amount
- Implement infer_trade_amount
- Implement is_balance method
- Restore the inventory logic
- Export price to string target
- Resolve #65 add bigdecimal ext to support round down and round up
- Refactor the txn balanced method
- Currency support precision and rounding
- Transaction balance support default and currency specific permission and rounding
- Implement commodities list and price list
- Impl FIFO and FILO strategy
- Record currency info into lots
- Format code
- Impl commodity lots detail
- Filter zero number lots
- Update dockerfile and use 8000
- Copy data folder from builder
- Update dockerfile and use 8000
- Rename docker image name
- I18n support, and switch in setting page
- Format code
- Use matches macro
- I18n the nav items
- Implement journal pagination
- Format code
- Implment Eq for those implemented PartialEq
- Implement Eq for structs
- Implement Eq for structs
- Implement Eq for structs
- Parallel check flow
- Cancel the previous running workflow
- Check if file content match zhang's syntax
- Document list item
- Group document by file path
- Format code
- Watch the entry path for incoming zhang's file candidate, and ignore unrelated files
- Text target support transaction cost
- Update commodity info when processing commodity directive
- New mantine ui
- Refact number representation
- Allow unused parse rule
- Set logarithmic for bar chart
- Show trx summary in vertical center
- Show summary color base on amount sign
- Implement transaction upload logic
- New section box ui implementation
- Align title and date range picker to oneline
- Store hide closed account flag into local storage
- Make clippy happy
- Component with oneline on sm page size
- Placeholder and fullscreen when on mobile
- Replace useState with useListState
- Upload screenshot picture
- Update readme
- Show hierachy account tree
- Align right for the blanace column
- Align last price date to end
- Pretty the link and tag badge
- Update nav icon
- Change system to async
- Handle sqlite process for close and commodities directive
- All directive support database cache now
- Format code
- Make clippy happy
- New account list restful api.
- Using actix-web instead of axum
- Account balance api
- Account journal api
- Get files name api
- Get/update file content api
- New journal api
- Journal item mobile ui
- Big decimal for sqlite type
- File edit and simple statistic
- New transaction api
- Statistic api implementation
- Enable log with default info level
- Wrap api data into wrapper
- Implement statistic api
- Report api
- Report income and expense rank
- Journal pagination
- Error api
- Document preview modal
- Document preview for document table
- Fmt code
- Fmt code
- Update base rust version
- #101 add basic info api
- #101 add log for the version
- #104 text to be translated
- Make clippy happy
- #101 add interval task of version report
- #101 add cli opt to control the report task
- #101 make clippy happ
- Suppress error threshold to 256
- Impl basic table view
- New graph ui and table ui
- Badge for journal type
- Fold useless code
- Show balance check
- Journal only support table view now
- Show journal preview icon when hover
- Fold useless code
- Use borrow trait to accept map key
- #111 add time logging for directive process function
- #111 add instruction for flamegraph
- #67 remove self implemented line info extractor
- Use memory database if path is not present
- Move sql operation into domain
- #108 sse MVP
- #109 only listen modify event
- #108 reload error and currency info when reload
- #108 store account into redux
- #108 show basic info and control by sse
- #108 remove version in nav
- #108 add line clamp for title
- Update example docker builder
- Create application folder
- Update example docker builder
- Update example docker builder
- Non root user
- Non root user
- Update readme.md
- Update screenshot
- Update readme.md
- Impl #113 journal data store in redux
- #116 simple setting page
- Format the code via prettier
- Update dev cargo build
- Update example
- Align three setting into oneline
- #126 extract transformer layout
- #126 make cargo check and clippy happy
- #126 add frontend feature to make bundled frontend as optional
- #126 support inline comment
- #126 support pad directive
- #126 support pushtag and poptag
- #126 separate project into multiple slight lib
- #126 add text transformer and exporter
- Tier display file tree
- How exporter append directive
- #126 extract appendableExporter from exporter, so the server can handle append operation.
- Rename append-directives method's name
- Apply function no need move to exporter
- Make fmt and clippy happy
- New error kind for ast
- Introduce the text file based transformer
- Impl grid document box
- #125 add error notification for new transaction
- Generate fixed id
- Bump openssl from 0.10.47 to 0.10.50
- Use fixed id for journal to make frontend rendering work
- Batch balance ui
- Bump h2 from 0.3.15 to 0.3.17
- Update code coverage step
- Update cache step
- Update cache step
- Update coverage step
- Make clippy happy
- Bump spin from 0.9.3 to 0.9.8
- Resolve #42 support wildcard operator in include directive
- Clean zhang pest file
- Rename parser mode
- Clean up zhang syntax and add some beancount directives
- Handle beancount push tag and pop tag directive
- Try to infer transformer by file extension
- Inline method to trait
- Reorg beancount directive variable name
- Combine pad and balance directive into one
- Make clippy happy
- Implement beancount exporter
- Extract time from meta
- Make clippy happy
- Move sql schema to seperated file
- Extract operations to centrelize database operation
- Return calculated statistic
- Display calculated amount in frontend
- Statistic box style
- Display label behind amount
- Extract metas
- Set max line width to 160
- Move some db operations into operations
- Report graph style and report statistic style
- Colorful statistic box
- Colorful statistic box
- Store error in database
- Centrelize insert option operation and add test
- Make clippy happy
- Make fmt happy
- Store default options into DB
- Extract constants
- Extract insert meta operation
- Commodity should get precision from default options
- Meta precision should have higher priority
- Raise error on close directive
- Make fmt happy
- Extract operation of closing an account
- Extract text enum macro
- Extract account status enum
- Extract meta type enum
- Update code coverage badge
- Modify balance check narration
- Bold payee
- Should amount in single account page as currency format and align right
- Impl balance check and pad UI in line and detail preview
- Bump to 0.1.0-alpha.1

### Fixed

- Fix doc test
- To text trait
- Fix custom to text trait
- Fix
- Fix chinese name
- #9 correct include path
- Parser support no flag
- Fix yarn install command
- Wechat importer and frontend demo
- Frontend uri without extension map to index.html for frontend router
- Git ignore
- Transaction support non-amount posting
- Chinese word would not be escaped
- Refresh document info when reloading.
- New transaction button text
- Update errors when reloading
- Update current snapshot after balance check
- Show current month statistic
- Distance account filter should be start with
- Sort account
- Production do not hard code url prefix
- Disable for oneline
- Fix mkdir nested folder
- Plugin test
- Fix account parent method
- Parser of posting cost time and to text target test
- Parse option directive
- Update frontend build folder
- Fix typescript warning
- Fix bin name
- Fix docker file
- Amount number in statistic bar
- Chart registry all registerables
- Use [u8] as buffer to support binary file
- #37 null payee should return empty string
- Resolve #18 format zero as 0 instead of -0
- Fix test
- #67 use self-implement line col algorithm due to low speed of pest implement
- Null handler for posting unit
- Process instance access issue
- Move hook after condition judgement
- Canonicalize the entry path when storing
- Rounding key of commodity
- Use inferredUnit if unit is not defined
- Resolved #91 manually handle journal page data
- Handle duplicated page info
- Use function date to access old data
- Balance check style
- Report page ui issue
- Group the same summaries for same currency
- Line offset of uploading transaction document
- Full page height for the journal page
- Home graph total sign and enable logarithmic
- Tags and links for single account request
- Empty tags and links
- Empty tags and links on preview page
- Correct date getter from Date instance
- Newline issue in windows
- Chart height
- Check error implementation
- Fix issue regrading single document uploaded as multiple documents
- Document preview issue
- Restore transaction's balance check
- Test for acount open and close directive
- Should update commodity info given operating commodity and commodity directive
- Test for multiple file
- Navbar i18n
- Should distinct the documents
- Use the same db file when reloading
- Only open document preview modal for image document
- Disable user scalable
- #103 remove empty payee
- #104 add payee candidate into select item
- #102 fetch REAL type as string and convert it to bigdecimal
- #104 remove useless code
- #104 use raw state hook
- Use bigdecimal to calculate balance
- Report status group
- #2 set precision as 2 if not presented
- #126 remove comment test
- #126 add frontend feature flag when build release
- Align item in table into right
- Return invalid account error instead of empty given invalid account
- #126 no need to wrap in Data twice
- #126 use text exporter as default exporter
- #126 test case
- Fix Dockerfile
- Square box for images of document preview modal
- Insert document for transaction
- Unit test case
- Infer transformer via file extension
- Fix unit test
- Fix the order of latest account balance and load exporter based on extension
- Balance directive should have higher priority
- Should use sequence to get day's latest account balance
- Clean up beancount parser
- Price's amount should use real type
- Account balance sql
- Datetime col name for account balance and add empty test
- Should be passed given same operating currency and defined commodity

### Removed

- Remove lalrpop parser
- Remove lalrpop
- Remove useless import
- Remove old transaction
- Remove from str for inventory
- Remove useless mod file.
- Remove clippy and fmt warning
- Remove useless import
- Remove dbg!
- Remove pre-commit hook
- Remove build check for typescript
- Remove useless parse function
- Remove useless parse functiono
- Remove useless inventory mod
- Remove useless frontend project
- Remove lock file flag for cargo build
- Remove useless import
- Remove unused import
- Remove todo for report
- Remove useless import
- Remove error log for account balance check
- Remove useless import
- Remove dbg macro
- Remove unuse import
- Remove dbg macro
- Remove musl buld
- Remove musl buld
- Remove useless import
- Remove useless variable
- Remove useless import
- Remove useless import
- Remove useless import
- Remove import
- Remove useless nav item
- Remove test notifications
- Remove uesless import
- Remove useless icon import
- Remove pre commit hook
- Remove dbg statement
- Remove rust hook
- Remove context cache in ledger entity, cuz it is used tempfile as store.
- Remove useless import
- Remove graphql related code
- Remove graphql related dependency
- Remove useless test code
- Remove graphql dependency
- Remove useless import
- Remove codemirror to make it lightweight
- Remove capover file
- Remove jotai and implement with redux
- Remove useless code
- Remove useless process context
- Remove useless load from str
- Remove some unwrap
- Remove useless dependencies
- Remove some done todo
- Remove useless import
- Remove comment
- Remove trailing space in open directive
- Remove filter condition
- Remove useless in-memory options and configs store

[0.1.10]: https://github.com///compare/v0.1.9..v0.1.10
[0.1.9]: https://github.com///compare/v0.1.8..v0.1.9
[0.1.8]: https://github.com///compare/v0.1.8-beta.3..v0.1.8
[0.1.8-beta.3]: https://github.com///compare/v0.1.8-beta.2..v0.1.8-beta.3
[0.1.8-beta.2]: https://github.com///compare/v0.1.8-beta.1..v0.1.8-beta.2
[0.1.8-beta.1]: https://github.com///compare/v0.1.7..v0.1.8-beta.1
[0.1.7]: https://github.com///compare/v0.1.6..v0.1.7
[0.1.6]: https://github.com///compare/v0.1.5..v0.1.6
[0.1.5]: https://github.com///compare/v0.1.4..v0.1.5
[0.1.4]: https://github.com///compare/v0.1.3..v0.1.4
[0.1.3]: https://github.com///compare/v0.1.2..v0.1.3
[0.1.2]: https://github.com///compare/v0.1.1..v0.1.2
[0.1.1]: https://github.com///compare/v0.1.0..v0.1.1
[0.1.0]: https://github.com///compare/v0.1.0-beta.6..v0.1.0
[0.1.0-beta.6]: https://github.com///compare/v0.1.0-beta.5..v0.1.0-beta.6
[0.1.0-beta.5]: https://github.com///compare/v0.1.0-beta.4..v0.1.0-beta.5
[0.1.0-beta.4]: https://github.com///compare/v0.1.0-beta.3..v0.1.0-beta.4
[0.1.0-beta.3]: https://github.com///compare/v0.1.0-beta.2..v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com///compare/v0.1.0-alpha.5..v0.1.0-beta.2
[0.1.0-alpha.5]: https://github.com///compare/v0.1.0-alpha.4..v0.1.0-alpha.5
[0.1.0-alpha.3]: https://github.com///compare/v0.1.0-alpha.2..v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com///compare/v0.1.0-alpha.1..v0.1.0-alpha.2

<!-- generated by git-cliff -->
