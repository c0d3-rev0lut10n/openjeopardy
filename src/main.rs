/*	Copyright (c) 2023 Laurenz Werner
	
	This file is part of OpenJeopardy.
	
	OpenJeopardy is free software: you can redistribute it and/or modify
	it under the terms of the GNU General Public License as published by
	the Free Software Foundation, either version 3 of the License, or
	(at your option) any later version.
	
	OpenJeopardy is distributed in the hope that it will be useful,
	but WITHOUT ANY WARRANTY; without even the implied warranty of
	MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
	GNU General Public License for more details.
	
	You should have received a copy of the GNU General Public License
	along with OpenJeopardy.  If not, see <http://www.gnu.org/licenses/>.
*/

#[macro_use]
extern crate lazy_static;

use crate::AnswerResult::*;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use actix_web::{get, post, delete, web, App, HttpRequest, HttpResponse, HttpServer, Responder, web::Redirect};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD as BASE64};
use moka::future::Cache;
use serde::Deserialize;
use regex::Regex;

const SERVER_ADDRESS: &str = "0.0.0.0";
const SERVER_PORT: u16 = 4242;

const SITE_HEADER: &str = "
<html>
	<head>
		<title>Jeopardy</title>
		<style>
			.buzzer {
				background-color: #ee2210;
				border: none;
				border-radius: 42px;
				color: #fff;
				width: 90vw;
				height: 95vh;
				text-decoration: none;
				margin-top: 10px;
				margin-bottom: 10px;
				margin-left: 5vw;
				margin-right: 5vw;
				cursor: pointer;
				font-size: 8vw;
			}
			.regular {
				background-color: #3a5eff;
				border: none;
				color: #fff;
				width: 90vw;
				height: 100px;
				text-decoration: none;
				margin-top: 10px;
				margin-bottom: 10px;
				margin-left: 3vw;
				margin-right: 3vw;
				cursor: pointer;
			}
			.pad {
				margin-left: 3vw;
				margin-right: 3vw;
			}
			input[type=text] {
				margin-left: 3vw;
				margin-right: 3vw;
				width: 90vw;
			}
		</style>
	</head>
	<body>";
const SITE_FOOTER: &str = "	</body>
</html>";

lazy_static! {
	static ref IS_VALID_NAME: Regex = Regex::new("^[0-9a-zA-Z_-]+$").unwrap();

}

#[derive(Deserialize)]
struct RegisterQuery {
	name: String,
}

#[derive(Deserialize)]
struct AnswerQuery {
	c: u8, // category (0-4)
	a: u8, // answer (0-4)
	value: Option<u16>, // set value for double jeopardies
	rating: Option<Rating>, // when a question and a player are active, this decides about the points given
}

#[derive(Deserialize)]
struct AdminQuery {
	setstate: Option<u8>, // set state: Registration or BuzzerActive
	reset: Option<bool>, // reset entire game, kicking all players
	player: Option<u8>, // select a player that shall be active now
}

#[derive(Clone, Deserialize, Debug)]
struct Answers {
	categories: Vec<Category>,
	#[serde(skip)]
	active_player: Option<u8>,
}

#[derive(Clone, Deserialize, Debug)]
struct Category {
	name: String,
	answers: Vec<Answer>,
}

#[derive(Clone, Deserialize, Debug)]
struct Answer {
	task: Task,
	points: u16,
	double: bool,
	#[serde(skip)]
	tries: Option<Vec<Try>>,
}

#[derive(Clone, Deserialize, Debug)]
enum Task {
	Picture(String),
	Text(String),
}

#[allow(non_camel_case_types)] // this is parsed from query string, which is lowercase by default
#[derive(Clone, Deserialize)]
enum Rating {
	positive,
	neutral,
	negative,
}

#[derive(Clone)]
struct Board {
	rows: Answers,
	players: Vec<Player>,
}

#[derive(Clone, Debug)]
struct Player {
	name: String,
	points: i32,
}

#[derive(Clone)]
struct ActivePlayer {
	id: u8,
}

#[derive(Clone, Debug)]
struct Try {
	player: String,
	try_result: AnswerResult,
}

#[derive(Clone, Debug)]
enum AnswerResult {
	positive(u16),
	negative(u16),
	neutral
}

enum Status {
	Registration,
	BuzzerActive,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
	let args: Vec<String> = std::env::args().collect();
	let pwd = std::env::current_dir()?;
	let mut path = pwd.clone();
	if args.len() < 2 { panic!("Provide a file to read the questions from!"); }
	path.push(&args[1]);
	
	let answers_file = fs::read_to_string(path.clone());
	if answers_file.is_err() { panic!("Could not parse data file!"); }
	
	let mut answers: Arc<RwLock<Answers>> = Arc::new(RwLock::new(serde_json::from_str(&answers_file.unwrap()).expect("Data file structure invalid!")));
	//answers.categories[0].answers[1].tries = Some(vec![Try {player: "bad".to_string(), try_result: AnswerResult::negative(100)}, Try {player: "42".to_string(), try_result: AnswerResult::positive(100)}]);
	//answers.categories[0].answers[1].tries.as_mut().unwrap().push(Try {player: "42".to_string(), try_result: AnswerResult::positive(10)});
	
	let status = Arc::new(RwLock::new(Status::Registration));
	let ip_cache = Cache::<String, String>::builder().build();
	let players = Arc::new(RwLock::new(Vec::<Player>::new()));
	let active_player = Arc::new(RwLock::new(ActivePlayer{id: 0}));
	HttpServer::new(move || {
		App::new()
			.app_data(web::Data::new(status.clone()))
			.app_data(web::Data::new(ip_cache.clone()))
			.app_data(web::Data::new(pwd.clone()))
			.app_data(web::Data::new(answers.clone()))
			.app_data(web::Data::new(players.clone()))
			.app_data(web::Data::new(active_player.clone()))
			.service(register)
			.service(buzz)
			.service(admin)
			.service(splash)
			.service(buzzer)
			.service(get_answer)
	})
	.bind((SERVER_ADDRESS, SERVER_PORT))?
	.run()
	.await
}

#[get("/register")]
async fn register(req: HttpRequest, query: web::Query<RegisterQuery>, status: web::Data<Arc<RwLock<Status>>>, ip_cache: web::Data<Cache<String, String>>, players: web::Data<Arc<RwLock<Vec<Player>>>>) -> impl Responder {
	match *(status.read().unwrap()) {
		Status::Registration => {},
		_ => {
			return HttpResponse::BadRequest().body("Game has already started! Try again later.");
		}
	};
	if !IS_VALID_NAME.is_match(&query.name) {
		return HttpResponse::BadRequest().body("Invalid name".as_bytes());
	}
	let ip: String = match req.peer_addr() {
		Some(res) => format!("{}", res.ip()),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	ip_cache.insert(ip.clone(), query.name.clone()).await;
	let mut players = players.write().unwrap();
	players.push(Player {
		name: query.name.to_string(),
		points: 0
	});
	
	println!("{} registered using name \"{}\"", ip, query.name);
	HttpResponse::TemporaryRedirect().insert_header(("location", "/buzzer")).finish()
}

#[get("/buzz")]
async fn buzz(req: HttpRequest, ip_cache: web::Data<Cache<String, String>>) -> impl Responder {
	let ip: String = match req.peer_addr() {
		Some(res) => format!("{}", res.ip()),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	let name = ip_cache.get(&ip).await;
	if name.is_none() {
		return HttpResponse::BadRequest().body("Not registered".as_bytes());
	}
	println!("{} buzzered!", name.unwrap());
	HttpResponse::TemporaryRedirect().insert_header(("location", "/buzzer")).finish()
}

#[get("/answer")]
async fn get_answer(req: HttpRequest, query: web::Query<AnswerQuery>, pwd: web::Data<PathBuf>, answers: web::Data<Arc<RwLock<Answers>>>, players: web::Data<Arc<RwLock<Vec<Player>>>>, active_player: web::Data<Arc<RwLock<ActivePlayer>>>) -> impl Responder {
	let ip = match req.peer_addr() {
		Some(res) => res.ip(),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	if !ip.is_loopback() {
		return HttpResponse::Unauthorized().body("Not an admin".as_bytes());
	}
	let mut path = PathBuf::clone(&pwd);
	path.push("answer.html");
	let answer_page_file = fs::read_to_string(&path);
	if answer_page_file.is_err() { return HttpResponse::InternalServerError().body("Could not parse answer.html in your PWD".as_bytes()) }
	
	let mut answer_page = answer_page_file.unwrap();
	
	let mut answers = answers.write().unwrap();
	
	let answer = &answers.categories[query.c as usize].answers[query.a as usize];
	
	let mut answer_string = match &answer.task {
		Task::Text(text) => {
			text.to_string()
		}
		Task::Picture(link) => {
			link.to_string() // TODO!
		}
	};
	if answer.double {
		answer_string = answer_string + " (DOUBLE)";
	}
	answer_page = answer_page.replace("CONTENT", &answer_string);
	answer_page = answer_page.replace("CAT", &query.c.to_string());
	answer_page = answer_page.replace("ANSWER", &query.a.to_string());
	
	if let Some(rating) = &query.rating {
		let active_player = active_player.read().unwrap();
		let mut player_list = players.write().unwrap();
		let player_name = if active_player.id as usize >= player_list.len() {
			format!("UNKNOWN No.{}", &(active_player.id + 1).to_string())
		}
		else {
			match rating {
				Rating::positive => {
					player_list[active_player.id as usize].points += answers.categories[query.c as usize].answers[query.a as usize].points as i32;
				}
				Rating::negative => {
					player_list[active_player.id as usize].points -= answers.categories[query.c as usize].answers[query.a as usize].points as i32;
				}
				Rating::neutral => {}
			};
			player_list[active_player.id as usize].name.clone()
		};
		if answers.categories[query.c as usize].answers[query.a as usize].tries.is_none() {
			answers.categories[query.c as usize].answers[query.a as usize].tries = Some(Vec::<Try>::new());
		}
		let answer_result = match rating {
			Rating::positive => { AnswerResult::positive(answers.categories[query.c as usize].answers[query.a as usize].points) }
			Rating::neutral => { AnswerResult::neutral }
			Rating::negative => { AnswerResult::negative(answers.categories[query.c as usize].answers[query.a as usize].points) }
		};
		answers.categories[query.c as usize].answers[query.a as usize].tries.as_mut().unwrap().push(Try {
			player: player_name,
			try_result: answer_result
		});
	}
	if let Some(value) = &query.value {
		answers.categories[query.c as usize].answers[query.a as usize].points = *value;
	}
	
	HttpResponse::Ok().body(answer_page.into_bytes())
}

#[get("/admin")]
async fn admin(req: HttpRequest, query: web::Query<AdminQuery>, pwd: web::Data<PathBuf>, answers: web::Data<Arc<RwLock<Answers>>>, players: web::Data<Arc<RwLock<Vec<Player>>>>, active_player: web::Data<Arc<RwLock<ActivePlayer>>>, status: web::Data<Arc<RwLock<Status>>>) -> impl Responder {
	let ip = match req.peer_addr() {
		Some(res) => res.ip(),
		None => return HttpResponse::InternalServerError().body("Could not get IP address".as_bytes())
	};
	if !ip.is_loopback() {
		return HttpResponse::Unauthorized().body("Not an admin".as_bytes());
	}
	if query.player.is_some() {
		let mut active_player = active_player.write().unwrap();
		*active_player = ActivePlayer { id: query.player.unwrap() };
	}
	if query.setstate.is_some() {
		let mut status = status.write().unwrap();
		*status = match query.setstate.unwrap() {
			0 => { Status::Registration },
			_ => { Status::BuzzerActive },
		};
	}
	let mut path = PathBuf::clone(&pwd);
	path.push("admin.html");
	let admin_page_file = fs::read_to_string(&path);
	if admin_page_file.is_err() { return HttpResponse::InternalServerError().body("Could not parse admin.html in your PWD".as_bytes()) }
	let mut admin_page = admin_page_file.unwrap();
	
	let answers = answers.read().unwrap();
	
	let mut i = 0;
	for category in &answers.categories {
		i = i + 1;
		admin_page = admin_page.replace(&format!("CAT{}", i), &category.name);
		let mut j = 0;
		for answer in &category.answers {
			j = j + 1;
			let mut text = String::new();
			if answer.tries.is_some() {
				let tries = answer.tries.clone().unwrap();
				for m_try in tries {
					text = match m_try.try_result {
						positive(points) => {
							text + "+ " + &m_try.player + " (" + &points.to_string() + ")<br>"
						}
						negative(points) => {
							text + "- " + &m_try.player + " (" + &points.to_string() + ")<br>"
						}
						neutral => {
							text + "0 " + &m_try.player + "<br>"
						}
					};
				}
			}
			else {
				text = answer.points.to_string()
			}
			admin_page = admin_page.replace(&format!("C{}F{}", i, j), &text);
		}
	}
	let player_list = players.read().unwrap();
	let mut i = 1;
	for player in player_list.iter() {
		admin_page = admin_page.replace(&format!("P{}", i), &format!("{}: {}", &player.name, &player.points.to_string()));
		i += 1;
	}
	let state = match *status.read().unwrap() {
		Status::Registration => 1,
		Status::BuzzerActive => 0
	};
	admin_page = admin_page.replace("STATE", &state.to_string());
	
	HttpResponse::Ok().body(admin_page.into_bytes())
}

#[get("/")]
async fn splash() -> impl Responder {
	let site = format!("{}<h1 class=\"pad\">Willkommen zum Jeopardy! Gib dir einen Namen und registriere dich!</h1>
	<form action=\"/register\">
		<input type=\"text\" id=\"name\" name=\"name\">
		<input type=\"submit\" class=\"regular\" value=\"Registrieren\">
	</form>{}", SITE_HEADER, SITE_FOOTER).into_bytes();
	HttpResponse::Ok().body(site)
}

#[get("/buzzer")]
async fn buzzer() -> impl Responder {
	let site = format!("{}<form action=\"/buzz\">
			<input type=\"submit\" class=\"buzzer\" value=\"Buzzer!\">
		</form>{}", SITE_HEADER, SITE_FOOTER);
	let site = site.into_bytes();
	HttpResponse::Ok().body(site)
}
