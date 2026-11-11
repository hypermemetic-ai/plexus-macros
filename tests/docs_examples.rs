//! The code of `docs/*.md`. Each block of a page that names this file is
//! lines of it, and `tests/readme.rs` holds the page to them; the tests here
//! run what the pages say.

#![allow(dead_code)]

/// docs/activation.md: the type names the activation.
mod names {
    use futures::{stream, Stream};
    use plexus_core::plexus::{Activation, InProcessTransport};
    use plexus_macros::PlexusWire;

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub struct Ticket {
        pub id: String,
    }

    #[derive(Clone)]
    pub struct Tickets;

    /// Tickets.
    #[plexus_macros::activation]
    impl Tickets {
        /// Every ticket.
        pub async fn list(&self) -> impl Stream<Item = Ticket> + Send + 'static {
            stream::iter([Ticket { id: "T-1".into() }])
        }

        /// How many there are.
        #[plexus_macros::method]
        pub async fn total(&self) -> u64 {
            1
        }

        pub fn new() -> Self {
            Self
        }
    }

    #[tokio::test]
    async fn the_type_names_the_namespace_the_trait_and_the_client() {
        let tickets = Tickets::new();
        assert_eq!(tickets.namespace(), "tickets");
        assert_eq!(tickets.description(), "Tickets.");
        assert_eq!(tickets.methods(), vec!["list", "total", "schema"]);

        let by_its_trait: &dyn TicketsActivationSchema = &tickets;
        assert_eq!(by_its_trait.total().await, 1);

        let client = TicketsClient::new(InProcessTransport::new(tickets));
        assert_eq!(client.total().await.unwrap(), 1);
    }
}

/// docs/activation.md: arguments.
mod arguments {
    use plexus_core::plexus::Activation;

    #[derive(Clone)]
    pub struct StorefrontActivation;

    #[plexus_macros::activation(
        namespace = "shop",
        version = "2.0.0",
        description = "Things for sale",
        long_description = "Every item the shop sells, and what it costs.",
    )]
    impl StorefrontActivation {
        /// Whether the shop is open.
        #[plexus_macros::method]
        pub async fn open(&self) -> bool {
            true
        }
    }

    #[test]
    fn the_arguments_reach_the_schema() {
        let schema = StorefrontActivation.plugin_schema();
        assert_eq!(schema.namespace, "shop");
        assert_eq!(schema.version, "2.0.0");
        assert_eq!(schema.description, "Things for sale");
        assert_eq!(schema.long_description.as_deref(), Some("Every item the shop sells, and what it costs."));
        let _client: StorefrontClient<()> = StorefrontClient::new(());
    }
}

/// docs/activation.md: a contract.
mod contract {
    use futures::{stream, Stream, TryStreamExt};
    use plexus_client::BoxStream;
    use plexus_core::plexus::InProcessTransport;

    /// Greets people.
    #[plexus_macros::activation]
    impl GreeterActivation {
        /// Greet someone by name.
        pub async fn greet(&self, name: String) -> impl Stream<Item = String> + Send + 'static;
    }

    pub struct Polite;

    impl Greeter for Polite {
        fn greet(&self, name: String) -> BoxStream<'static, String> {
            Box::pin(stream::iter([format!("Good day, {name}.")]))
        }
    }

    #[tokio::test]
    async fn an_implementation_is_served() {
        let activation: GreeterActivation<Polite> = Polite.server();
        let client = GreeterClient::new(InProcessTransport::new(activation));
        let said: Vec<String> = client.greet("Ada".into()).try_collect().await.unwrap();
        assert_eq!(said, ["Good day, Ada."]);
    }
}

/// docs/activation.md: `impl Trait for Type`.
mod trait_impl {
    use plexus_core::plexus::{Activation, InProcessTransport};

    #[allow(async_fn_in_trait)]
    pub trait Forge {
        async fn repos(&self) -> Vec<String>;
    }

    #[derive(Clone)]
    pub struct GitHub;

    /// GitHub, as a forge.
    #[plexus_macros::activation]
    impl Forge for GitHub {
        /// Its repositories.
        async fn repos(&self) -> Vec<String> {
            vec!["plexus-macros".into()]
        }
    }

    #[tokio::test]
    async fn the_trait_names_the_namespace_and_the_client() {
        assert_eq!(GitHub.namespace(), "forge");
        let client = ForgeClient::new(InProcessTransport::new(GitHub));
        assert_eq!(client.repos().await.unwrap(), ["plexus-macros"]);
    }
}

/// docs/activation.md: `served = written`.
mod served_written {
    use futures::{stream, Stream};
    use plexus_core::plexus::Activation;

    #[derive(Clone)]
    pub struct Index;

    #[plexus_macros::activation]
    impl Index {
        /// Every entry.
        pub async fn entries(&self) -> impl Stream<Item = String> + Send + 'static {
            stream::iter(["dune".to_string()])
        }
    }

    #[derive(Clone)]
    pub struct Book {
        title: String,
    }

    #[plexus_macros::activation]
    impl Book {
        /// The book's title.
        #[plexus_macros::method]
        pub async fn title(&self) -> String {
            self.title.clone()
        }
    }

    #[derive(Clone)]
    pub struct Shelf;

    /// A shelf of books.
    #[plexus_macros::activation(served = written)]
    impl Shelf {
        /// Not on `&self`: a helper.
        pub fn new() -> Self {
            Shelf
        }

        /// Not `pub`: a helper.
        fn size(&self) -> u32 {
            1
        }

        /// A `pub async fn` on `&self`: a method.
        pub async fn count(&self) -> u32 {
            self.size()
        }

        /// A `pub fn` on `&self`: a static child.
        pub fn index(&self) -> Index {
            Index
        }

        /// A `pub fn` on `&self` taking a name: a dynamic child.
        pub fn book(&self, name: &str) -> Option<Book> {
            (name == "dune").then(|| Book { title: name.to_string() })
        }
    }

    #[test]
    fn each_fn_is_what_its_signature_says() {
        let shelf = Shelf::new();
        assert_eq!(shelf.methods(), vec!["count", "schema"]);
        let roles: Vec<(String, String)> = shelf
            .plugin_schema()
            .methods
            .iter()
            .map(|m| (m.name.clone(), serde_json::to_value(&m.role).unwrap()["kind"].as_str().unwrap_or("?").to_string()))
            .collect();
        assert!(roles.iter().any(|(n, _)| n == "index"), "{roles:?}");
        assert!(roles.iter().any(|(n, _)| n == "book"), "{roles:?}");
    }
}

/// docs/methods.md: the six return forms.
mod forms {
    use futures::{stream, Stream, StreamExt, TryStreamExt};
    use plexus_client::Frame;
    use plexus_core::plexus::{Activation, InProcessTransport, Note, Progress};
    use plexus_macros::PlexusWire;

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub struct Ticket {
        pub id: String,
    }

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub enum GetError {
        NotFound { id: String },
    }

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub enum ImportStep {
        Chunk { done: u32, of: u32 },
    }

    #[derive(Clone)]
    pub struct Tickets;

    /// Tickets.
    #[plexus_macros::activation]
    impl Tickets {
        /// One item.
        #[plexus_macros::method]
        pub async fn total(&self) -> u64 {
            1
        }

        /// One item, or the activation's own error.
        #[plexus_macros::method]
        pub async fn get(&self, id: String) -> Result<Ticket, GetError> {
            if id == "T-1" { Ok(Ticket { id }) } else { Err(GetError::NotFound { id }) }
        }

        /// Many items.
        pub async fn list(&self) -> impl Stream<Item = Ticket> + Send + 'static {
            stream::iter([Ticket { id: "T-1".into() }])
        }

        /// Many items, each of which can be an error. An `Err` does not end the stream.
        pub async fn check(&self, ids: Vec<String>) -> impl Stream<Item = Result<Ticket, GetError>> + Send + 'static {
            stream::iter(ids).map(|id| if id == "T-1" { Ok(Ticket { id }) } else { Err(GetError::NotFound { id }) })
        }

        /// Many items, with progress of the default type, `Note`.
        pub async fn export(&self) -> impl Stream<Item = Progress<Ticket>> + Send + 'static {
            stream::iter([
                Progress::Update(Note { message: "reading".into(), percentage: Some(50.0) }),
                Progress::Item(Ticket { id: "T-1".into() }),
            ])
        }

        /// Many items, with progress of a declared type, and errors.
        pub async fn import(&self, source: String) -> impl Stream<Item = Result<Progress<Ticket, ImportStep>, GetError>> + Send + 'static {
            stream::iter([
                Ok(Progress::Update(ImportStep::Chunk { done: 1, of: 1 })),
                Ok(Progress::Item(Ticket { id: source })),
            ])
        }
    }

    #[tokio::test]
    async fn the_client_answers_each_form() -> Result<(), plexus_client::ClientError> {
        let client = TicketsClient::new(InProcessTransport::new(Tickets));

        // One item: `?` is the call failing, the value is the answer.
        let total: u64 = client.total().await?;
        assert_eq!(total, 1);

        // One item that can be an error: the activation's `Err` is a value.
        match client.get("T-2".into()).await? {
            Ok(ticket) => panic!("found {ticket:?}"),
            Err(GetError::NotFound { id }) => assert_eq!(id, "T-2"),
        }

        // Many items: a stream of `Result<item, ClientError>`.
        let all: Vec<Ticket> = client.list().try_collect().await?;
        assert_eq!(all.len(), 1);

        // An `Err` item is data, and the stream goes on past it.
        let checked: Vec<Result<Ticket, GetError>> = client.check(vec!["T-2".into(), "T-1".into()]).try_collect().await?;
        assert!(checked[0].is_err() && checked[1].is_ok());

        // Progress is dropped unless asked for.
        let imported: Vec<Result<Ticket, GetError>> = client.import("T-9".into()).try_collect().await?;
        assert_eq!(imported, [Ok(Ticket { id: "T-9".into() })]);

        // `.with_progress()` yields both, in the order they were sent.
        let frames: Vec<Frame<Result<Ticket, GetError>, ImportStep>> = client.import("T-9".into()).with_progress().try_collect().await?;
        assert_eq!(frames[0], Frame::Progress(ImportStep::Chunk { done: 1, of: 1 }));
        assert_eq!(frames[1], Frame::Item(Ok(Ticket { id: "T-9".into() })));

        // `.events()` yields the activation's aggregate, one variant per method.
        let events: Vec<TicketsEvent> = client.list().events().try_collect().await?;
        assert!(matches!(&events[0], TicketsEvent::List(Ticket { id }) if id == "T-1"));
        Ok(())
    }

    #[test]
    fn the_client_and_the_aggregate_are_named_from_the_activation() {
        type Client = TicketsClient<plexus_client::MemoryTransport>;
        assert_eq!(Client::NAMESPACE, "tickets");
        assert_eq!(Client::METHOD_LIST.qualified(), "tickets.list");
        assert_eq!(TicketsClient::at((), "desk.tickets").path(), "desk.tickets");

        let client = Client::new(plexus_client::MemoryTransport::new());
        let _held: TicketsListStream<'_> = client.list();

        let event = TicketsEvent::List(Ticket { id: "T-1".into() });
        assert_eq!(serde_json::to_string(&event).unwrap(), r#"{"method":"list","event":{"id":"T-1"}}"#);
        let event = TicketsEvent::Get(Err(GetError::NotFound { id: "T-2".into() }));
        assert_eq!(serde_json::to_string(&event).unwrap(), r#"{"method":"get","event":{"err":{"type":"not_found","id":"T-2"}}}"#);
    }

    #[test]
    fn the_schema_says_each_form() {
        let schema = serde_json::to_value(Tickets.plugin_schema()).unwrap();
        let method = |name: &str| schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap().clone();
        assert_eq!(method("total")["cardinality"], "one");
        assert!(method("total")["error"].is_null());
        assert_eq!(method("get")["cardinality"], "one");
        assert!(method("get")["error"].is_object());
        assert_eq!(method("list")["cardinality"], "many");
        assert!(method("export")["progress"].is_object());
        assert!(method("import")["progress"].is_object() && method("import")["error"].is_object());
        assert_eq!(method("total")["http_method"], "POST");
    }

    #[tokio::test]
    async fn a_result_travels_as_ok_or_err() {
        use serde_json::json;
        let tickets = plexus_client::Untyped::new(InProcessTransport::new(Tickets), "tickets");
        assert_eq!(tickets.call_one("total", json!({})).await.unwrap(), json!(1));
        assert_eq!(tickets.call_one("get", json!({ "id": "T-1" })).await.unwrap(), json!({ "ok": { "id": "T-1" } }));
        assert_eq!(
            tickets.call_one("get", json!({ "id": "T-2" })).await.unwrap(),
            json!({ "err": { "type": "not_found", "id": "T-2" } })
        );
    }
}

/// docs/methods.md: parameters and `#[method(...)]`.
mod method_arguments {
    use plexus_client::{ClientError, ParamProblem, RemoteError, Untyped};
    use plexus_core::plexus::{Activation, InProcessTransport};
    use serde_json::json;

    #[derive(Clone)]
    pub struct Notes;

    /// Notes.
    #[plexus_macros::activation]
    impl Notes {
        /// Find notes.
        #[plexus_macros::method(
            name = "search",
            group = "read",
            http_method = "GET",
            params(limit = "At most this many"),
            values(status = ["open", "closed"]),
            example(note = "the first ten open notes", params = r#"{"text": "plexus", "status": "open", "limit": 10}"#),
        )]
        pub async fn find(
            &self,
            /** Words to look for */ text: String,
            status: Option<String>,
            limit: Option<u32>,
        ) -> Vec<String> {
            let _ = (status, limit);
            vec![text]
        }
    }

    #[test]
    fn the_arguments_reach_the_schema() {
        let schema = serde_json::to_value(Notes.plugin_schema()).unwrap();
        let search = schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == "search").unwrap().clone();
        assert_eq!(search["description"], "Find notes.");
        assert_eq!(search["group"], "read");
        assert_eq!(search["http_method"], "GET");
        let props = &search["params"]["properties"];
        assert_eq!(props["text"]["description"].as_str().unwrap().trim(), "Words to look for");
        assert_eq!(props["limit"]["description"], "At most this many");
        assert_eq!(props["status"]["x-plexus-values"], json!(["open", "closed"]));
        assert_eq!(search["params"]["required"], json!(["text"]));
        assert_eq!(search["examples"][0]["note"], "the first ten open notes");
    }

    #[tokio::test]
    async fn a_bad_parameter_is_named() {
        let notes = Untyped::new(InProcessTransport::new(Notes), "notes");
        let refused = notes.call_one("search", json!({})).await.unwrap_err();
        assert!(matches!(
            refused,
            ClientError::Remote(RemoteError::InvalidParams { param: Some(param), problem: ParamProblem::Missing }) if param == "text"
        ));

        let refused = notes.call_one("search", json!({ "text": 7 })).await.unwrap_err();
        assert!(matches!(
            refused,
            ClientError::Remote(RemoteError::InvalidParams { param: Some(param), problem: ParamProblem::Malformed { .. } }) if param == "text"
        ));

        // A key that is no parameter is not looked at.
        assert_eq!(notes.call_one("search", json!({ "text": "a", "nope": 1 })).await.unwrap(), json!(["a"]));
    }
}

/// docs/children.md: children and the clients that reach them.
mod children {
    use futures::{stream, Stream, StreamExt};
    use plexus_core::plexus::{Activation, ChildRouter, DynamicHub, HubTransport};
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct Sun;

    /// The star.
    #[plexus_macros::activation]
    impl Sun {
        /// Its name.
        #[plexus_macros::method]
        pub async fn name(&self) -> String {
            "Sol".into()
        }
    }

    #[derive(Clone)]
    pub struct Planet {
        name: String,
    }

    /// One planet.
    #[plexus_macros::activation]
    impl Planet {
        /// Its name.
        #[plexus_macros::method]
        pub async fn name(&self) -> String {
            self.name.clone()
        }
    }

    #[derive(Clone)]
    pub struct Solar;

    /// The solar system.
    #[plexus_macros::activation]
    impl Solar {
        /// How many planets.
        #[plexus_macros::method]
        pub async fn count(&self) -> u32 {
            2
        }

        /// The star: reached as `solar.sun`.
        #[plexus_macros::child]
        fn sun(&self) -> Sun {
            Sun
        }

        /// A planet by name: reached as `solar.<name>`.
        #[plexus_macros::child(list = planet_names, search = find_planets)]
        fn planet(&self, name: &str) -> Option<Planet> {
            ["earth", "mars"].contains(&name).then(|| Planet { name: name.to_string() })
        }

        fn planet_names(&self) -> impl Stream<Item = String> + Send + '_ {
            stream::iter(["earth".to_string(), "mars".to_string()])
        }

        fn find_planets(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
            let query = query.to_string();
            stream::iter(["earth".to_string(), "mars".to_string()].into_iter().filter(move |n| n.contains(&query)))
        }
    }

    #[tokio::test]
    async fn a_client_reaches_every_child() -> Result<(), plexus_client::ClientError> {
        let service = Arc::new(DynamicHub::new("svc").register(Solar));
        let solar = SolarClient::new(HubTransport::anonymous(service));

        assert_eq!(solar.count().await?, 2);
        assert_eq!(solar.sun().name().await?, "Sol"); // solar.sun.name
        assert_eq!(solar.planet("mars").name().await?, "mars"); // solar.mars.name
        Ok(())
    }

    #[tokio::test]
    async fn children_are_in_the_schema_by_role() {
        let schema = serde_json::to_value(Solar.plugin_schema()).unwrap();
        let role = |name: &str| schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap()["role"].clone();
        assert_eq!(role("sun")["kind"], "static_child", "{}", role("sun"));
        assert_eq!(role("planet")["kind"], "dynamic_child", "{}", role("planet"));
        assert_eq!(role("planet")["list_method"], "planet_names");
        assert_eq!(role("planet")["search_method"], "find_planets");
        let sun = schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == "sun").unwrap();
        assert_eq!(sun["description"], "The star: reached as `solar.sun`.");
        assert_eq!(Solar.methods(), vec!["count", "schema"]);

        let names: Vec<String> = Solar.list_children().await.unwrap().collect::<Vec<_>>().await;
        assert_eq!(names, ["sun", "earth", "mars"]);
    }
}

/// docs/wire-types.md.
mod wire_types {
    use plexus_macros::PlexusWire;

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub struct Point {
        pub x: i32,
        #[serde(default)]
        pub y: i32,
    }

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    pub enum CountEvent {
        Tick { n: u32 },
        AllDone,
    }

    #[derive(Debug, Clone, PartialEq, PlexusWire)]
    #[serde(untagged)]
    pub enum Either {
        Number(u32),
        Word(String),
    }

    #[test]
    fn a_wire_type_is_read_written_and_described() {
        assert_eq!(serde_json::to_string(&CountEvent::Tick { n: 2 }).unwrap(), r#"{"type":"tick","n":2}"#);
        assert_eq!(serde_json::to_string(&CountEvent::AllDone).unwrap(), r#"{"type":"all_done"}"#);
        assert_eq!(serde_json::from_str::<Point>(r#"{"x":1}"#).unwrap(), Point { x: 1, y: 0 });
        assert_eq!(serde_json::to_string(&Either::Word("a".into())).unwrap(), r#""a""#);
        let schema = serde_json::to_value(schemars::schema_for!(Point)).unwrap();
        assert_eq!(schema["title"], "Point");
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, plexus_macros::Deserialize, schemars::JsonSchema)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum TicketEvent {
        Health { version: String, count: u64 },
        Ready,
    }

    #[test]
    fn an_event_is_read_whichever_key_comes_first() {
        let first: TicketEvent = serde_json::from_str(r#"{"type":"health","version":"1","count":2}"#).unwrap();
        let last: TicketEvent = serde_json::from_str(r#"{"version":"1","count":2,"type":"health"}"#).unwrap();
        assert_eq!(first, last);
    }

    use plexus_core::plexus::Handle;
    use plexus_macros::HandleEnum;
    use uuid::Uuid;

    pub const NOTES_PLUGIN_ID: Uuid = uuid::uuid!("550e8400-e29b-41d4-a716-446655440000");

    #[derive(Debug, Clone, PartialEq, HandleEnum)]
    #[handle(plugin_id = "NOTES_PLUGIN_ID", version = "1.0.0")]
    pub enum NotesHandle {
        #[handle(method = "note", table = "notes", key = "id")]
        Note { note_id: String },
        #[handle(method = "message")]
        Message { message_id: String, role: String },
    }

    #[test]
    fn a_handle_goes_out_and_comes_back() {
        let note = NotesHandle::Note { note_id: "n-1".into() };
        let handle: Handle = note.to_handle();
        assert_eq!(handle.method, "note");
        assert_eq!(handle.meta, ["n-1"]);
        assert_eq!(NotesHandle::try_from(&handle).unwrap(), note);
        assert!(note.resolution_params().is_some());
    }
}

/// docs/wire-types.md: `PlexusRequest`.
mod request {
    use futures::{stream, Stream};
    use plexus_core::plexus::Activation;
    use plexus_core::request::PlexusRequest;
    use plexus_macros::PlexusRequest;

    #[derive(PlexusRequest)]
    pub struct AppRequest {
        /// The session cookie. Not an `Option`: a call without it is refused.
        #[from_cookie("access_token")]
        token: String,
        #[from_header("origin")]
        origin: Option<String>,
        #[from_peer]
        peer: Option<std::net::SocketAddr>,
        #[from_auth_context]
        auth: Option<plexus_core::plexus::AuthContext>,
    }

    #[derive(Clone)]
    pub struct App;

    /// An app.
    #[plexus_macros::activation(request = AppRequest)]
    impl App {
        /// Who the cookie says is calling.
        pub async fn me(&self, #[activation_param] token: String) -> impl Stream<Item = String> + Send + 'static {
            stream::iter([token])
        }

        /// Answers without reading the request.
        #[plexus_macros::method(request = ())]
        pub async fn health(&self) -> bool {
            true
        }
    }

    #[test]
    fn the_request_is_described() {
        let schema = AppRequest::request_schema().unwrap();
        assert_eq!(schema["properties"]["token"]["x-plexus-source"], serde_json::json!({ "from": "cookie", "key": "access_token" }));
        assert_eq!(schema["properties"]["origin"]["x-plexus-source"], serde_json::json!({ "from": "header", "key": "origin" }));
        assert_eq!(schema["properties"]["peer"]["x-plexus-source"], serde_json::json!({ "from": "derived" }));
        assert_eq!(schema["required"], serde_json::json!(["token"]));
        assert!(App.plugin_schema().request.is_some());
    }
}

/// docs/callers.md.
mod callers {
    use futures::{stream, Stream};
    use plexus_core::plexus::{Activation, AuthContext, PlexusError, Reach};
    use plexus_macros::PlexusRequest;

    #[derive(PlexusRequest)]
    pub struct DeskRequest {
        #[from_auth_context]
        auth: Option<AuthContext>,
    }

    #[derive(Debug, Clone)]
    pub struct Member {
        pub id: String,
    }

    #[derive(Clone)]
    pub struct Desk;

    impl Desk {
        async fn member(&self, auth: &AuthContext) -> Result<Member, PlexusError> {
            Ok(Member { id: auth.user_id.clone() })
        }
    }

    /// A help desk.
    #[plexus_macros::activation(
        request = DeskRequest,
        auth = "mixed",
        roles(
            viewer = ["get", "mine"],
            editor = [viewer, "close"],
        ),
    )]
    impl Desk {
        /// Open to everyone.
        #[plexus_macros::method(public)]
        pub async fn hours(&self) -> String {
            "9 to 5".into()
        }

        /// One ticket. The gate decides the call where `id` is.
        #[plexus_macros::method(target = "id")]
        pub async fn get(&self, id: String) -> String {
            id
        }

        /// The caller, required: an anonymous call is refused before the body.
        #[plexus_macros::method]
        pub async fn whoami(&self, auth: &AuthContext) -> String {
            auth.user_id.clone()
        }

        /// The caller, when there is one.
        #[plexus_macros::method]
        pub async fn greeting(&self, auth: Option<&AuthContext>) -> String {
            match auth {
                Some(auth) => format!("hello, {}", auth.user_id),
                None => "hello".into(),
            }
        }

        /// A value made from the caller by a resolver.
        #[plexus_macros::method]
        pub async fn close(&self, #[from_auth(self.member)] member: Member, id: String) -> String {
            format!("{} closed {id}", member.id)
        }

        /// Where the caller holds this method's grant.
        pub async fn mine(&self, reach: Reach) -> impl Stream<Item = String> + Send + 'static {
            stream::iter(reach.held_on().iter().map(|scope| scope.to_string()).collect::<Vec<_>>())
        }

        /// Acts on the whole service: admitted only where its grant is held on `*`.
        #[plexus_macros::method(everywhere)]
        pub async fn purge(&self) -> u32 {
            0
        }

        /// Needs both scopes, where the service has a scope registry.
        #[plexus_macros::method(scope = "desk.export", scope = "desk.read")]
        pub async fn export(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn what_is_declared_reaches_the_schema() {
        let schema = Desk.plugin_schema();
        let method = |name: &str| schema.methods.iter().find(|m| m.name == name).unwrap();

        assert!(method("hours").public);
        assert_eq!(method("get").target.as_deref(), Some("id"));
        assert_eq!(method("get").grant.as_deref(), Some("desk.get"));
        assert!(method("purge").everywhere);
        assert!(method("export").requires_credential.is_some());

        assert_eq!(schema.roles["viewer"], ["desk.get", "desk.mine"]);
        assert_eq!(schema.roles["editor"], ["desk.get", "desk.mine", "desk.close"]);

        // What the server passes is no parameter of the call.
        let params = |name: &str| -> Vec<String> {
            let Some(params) = method(name).params.as_ref() else { return Vec::new() };
            let params = serde_json::to_value(params).unwrap();
            params["properties"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
        };
        assert!(params("whoami").is_empty());
        assert!(params("mine").is_empty());

        // A resolver's value and the request's fields are marked as not the caller's.
        let close = serde_json::to_value(method("close").params.as_ref().unwrap()).unwrap();
        assert_eq!(close["properties"]["member"]["x-plexus-source"], serde_json::json!({ "from": "auth", "resolver": "self.member" }));
        assert_eq!(close["properties"]["auth"]["x-plexus-source"], serde_json::json!({ "from": "derived" }));
        assert_eq!(close["required"], serde_json::json!(["id"]));
        assert_eq!(close["properties"]["id"]["description"], "The id parameter");
    }

    #[tokio::test]
    async fn a_required_caller_is_refused_when_missing() {
        use plexus_client::{ClientError, RemoteError, Untyped};
        use plexus_core::plexus::InProcessTransport;
        let desk = Untyped::new(InProcessTransport::new(Desk), "desk");
        let refused = desk.call_one("whoami", serde_json::json!({})).await.unwrap_err();
        assert!(matches!(refused, ClientError::Remote(RemoteError::Unauthenticated)), "{refused:?}");
        let refused = desk.call_one("close", serde_json::json!({ "id": "T-1" })).await.unwrap_err();
        assert!(matches!(refused, ClientError::Remote(RemoteError::Unauthenticated)), "{refused:?}");
        assert_eq!(desk.call_one("greeting", serde_json::json!({})).await.unwrap(), "hello");
        assert_eq!(desk.call_one("hours", serde_json::json!({})).await.unwrap(), "9 to 5");
    }
}

/// docs/deprecation.md.
#[allow(deprecated)]
mod deprecation {
    use plexus_core::plexus::Activation;

    #[derive(Clone)]
    pub struct Clock;

    /// A clock.
    #[plexus_macros::activation]
    impl Clock {
        /// The time, in seconds.
        #[deprecated(since = "0.5.0", note = "use `now` instead")]
        #[plexus_macros::removed_in("0.6.0")]
        #[plexus_macros::method]
        pub async fn time(&self) -> u64 {
            0
        }

        /// The time, in milliseconds.
        #[plexus_macros::method]
        pub async fn now(
            &self,
            #[deprecated(since = "0.5.0", note = "the zone is always UTC")] zone: Option<String>,
        ) -> u64 {
            let _ = zone;
            0
        }
    }

    #[test]
    fn deprecation_reaches_the_schema() {
        let schema = serde_json::to_value(Clock.plugin_schema()).unwrap();
        let method = |name: &str| schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap().clone();
        assert_eq!(
            method("time")["deprecation"],
            serde_json::json!({ "since": "0.5.0", "removed_in": "0.6.0", "message": "use `now` instead" })
        );
        let zone = method("now")["params_meta"].as_array().unwrap().iter().find(|p| p["name"] == "zone").unwrap().clone();
        assert_eq!(zone["deprecation"]["removed_in"], "unspecified");
    }
}

/// docs/methods.md: `Input<T>`.
mod input {
    use futures::{stream, StreamExt};
    use plexus_core::plexus::{Activation, Input};

    #[derive(Clone)]
    pub struct Ingest;

    /// Takes tokens.
    #[plexus_macros::activation]
    impl Ingest {
        /// Count a run's tokens as they arrive.
        #[plexus_macros::method(input_window = 256)]
        pub async fn count(&self, run: String, tokens: Input<String>) -> u32 {
            let _ = run;
            tokens.into_stream().count().await as u32
        }
    }

    #[tokio::test]
    async fn the_input_is_a_stream_and_no_parameter() {
        let schema = serde_json::to_value(Ingest.plugin_schema()).unwrap();
        let count = schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == "count").unwrap().clone();
        assert!(count["input"].is_object(), "{count}");
        assert!(count["params"]["properties"].get("tokens").is_none(), "{count}");

        let on_the_trait: &dyn IngestActivationSchema = &Ingest;
        assert_eq!(on_the_trait.count("r1".into(), stream::iter(["a".to_string(), "b".to_string()]).boxed()).await, 2);

        // The client takes any stream of the items.
        let client = IngestClient::new(plexus_client::MemoryTransport::new());
        let _ = client.count("r1".into(), stream::iter(["a".to_string()]));
    }
}

/// docs/children.md: an activation that names its parent.
mod parent {
    use futures::{stream, Stream, StreamExt};
    use plexus_core::plexus::{Activation, Parent};

    #[derive(Clone)]
    pub struct Shop;

    /// A shop.
    #[plexus_macros::activation]
    impl Shop {
        /// What it sells.
        pub async fn items(&self) -> impl Stream<Item = String> + Send + 'static {
            stream::iter(["flour".to_string()])
        }
    }

    #[derive(Clone)]
    pub struct Restock;

    /// Restocks the shop it is grafted onto.
    #[plexus_macros::activation(parent = ShopClient)]
    impl Restock {
        /// What is running low.
        pub async fn low(&self, parent: Parent<ShopClient>, below: u32) -> impl Stream<Item = String> + Send + 'static {
            let items: Vec<String> = parent.items().filter_map(|item| async move { item.ok() }).collect().await;
            stream::iter(items.into_iter().take(below as usize))
        }
    }

    #[test]
    fn what_it_calls_of_its_parent_is_in_its_schema() {
        let schema = Restock.plugin_schema();
        let needs = schema.needs.unwrap();
        assert_eq!(needs.iter().map(|need| need.method.as_str()).collect::<Vec<_>>(), ["shop.items"]);
        assert!(!needs[0].hash.is_empty());

        let low = serde_json::to_value(schema.methods.iter().find(|m| m.name == "low").unwrap()).unwrap();
        assert!(low["params"]["properties"].get("parent").is_none(), "{low}");
        assert!(low["params"]["properties"].get("below").is_some(), "{low}");
    }
}

/// docs/methods.md: `typed_params`.
mod typed_params {
    use plexus_client::{ClientError, RemoteError, Untyped};
    use plexus_core::plexus::InProcessTransport;
    use serde_json::json;

    #[derive(Clone)]
    pub struct Ledger;

    /// A ledger.
    #[plexus_macros::activation(typed_params)]
    impl Ledger {
        /// Entries, newest first.
        #[plexus_macros::method]
        pub async fn entries(&self, limit: Option<u32>) -> Vec<u32> {
            (0..limit.unwrap_or(2)).collect()
        }
    }

    #[tokio::test]
    async fn a_key_that_is_no_parameter_is_refused() {
        let ledger = Untyped::new(InProcessTransport::new(Ledger), "ledger");
        assert_eq!(ledger.call_one("entries", json!({ "limit": 1 })).await.unwrap(), json!([0]));
        let refused = ledger.call_one("entries", json!({ "limt": 1 })).await.unwrap_err();
        assert!(matches!(refused, ClientError::Remote(RemoteError::InvalidParams { .. })), "{refused:?}");
        assert!(refused.to_string().contains("unknown field `limt`; ledger.entries takes: limit"), "{refused}");
    }
}

/// docs/children.md: a peer slot.
mod peer {
    use futures::TryStreamExt;
    use plexus_core::plexus::peer::{Peer, PeerError};
    use plexus_core::plexus::{DynamicHub, HubTransport};
    use serde_json::json;
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct Greeter;

    /// Says hello.
    #[plexus_macros::activation]
    impl Greeter {
        /// Hello.
        #[plexus_macros::method]
        pub async fn hello(&self, name: String) -> String {
            format!("hello, {name}")
        }
    }

    #[derive(Clone)]
    pub struct Lobby;

    /// A lobby.
    #[plexus_macros::activation]
    impl Lobby {
        /// How many greeters.
        #[plexus_macros::method]
        pub async fn count(&self) -> u32 {
            1
        }

        /// A greeter by name, which may be away.
        #[plexus_macros::peer]
        pub async fn greeter(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> {
            match name {
                "here" => Ok(Box::new(Greeter)),
                _ => Err(PeerError::NotFound { name: name.into() }),
            }
        }
    }

    #[tokio::test]
    async fn a_peer_is_reached_by_its_slot_and_its_name() {
        let service = Arc::new(DynamicHub::new("svc").register(Lobby));
        let lobby = LobbyClient::new(HubTransport::anonymous(service));

        let here = lobby.greeter("here");
        assert_eq!(here.path(), "lobby.greeter.here");
        let said: Vec<serde_json::Value> = here.call("hello", json!({ "name": "Ada" })).try_collect().await.unwrap();
        assert_eq!(said, [json!("hello, Ada")]);

        assert!(lobby.greeter("away").call_one("hello", json!({ "name": "Ada" })).await.is_err());

        use plexus_core::plexus::Activation;
        let schema = serde_json::to_value(Lobby.plugin_schema()).unwrap();
        let slot = schema["methods"].as_array().unwrap().iter().find(|m| m["name"] == "greeter").unwrap();
        assert_eq!(slot["role"], json!({ "kind": "static_child", "peer": true }));
    }
}
