//! Application-owned progress. A downloaded file is never an installed task.
use crate::transfer::{DownloadFile, Phase, Progress};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Task {
    pub title: String,
    pub stage: String,
    pub transfer: Option<Progress>,
    pub component: String,
    pub active: bool,
    pub error: Option<String>,
    pub details: VecDeque<String>,
    pub files: Vec<FileProgress>,
    finished: Option<Instant>,
    expired: bool,
}
#[derive(Clone, Debug)]
pub struct FileProgress {
    pub file: DownloadFile,
    /// None means announced but not started; complete requires verified bytes.
    pub phase: Option<Phase>,
    pub completed: u64,
}
impl FileProgress {
    pub fn label(&self) -> &'static str {
        match self.phase {
            None | Some(Phase::Waiting) => "等待",
            Some(Phase::Complete) => "已完成",
            Some(Phase::Verifying) => "校验中",
            Some(Phase::Switching | Phase::Restarting) => "重试中",
            _ => "下载中",
        }
    }
}
impl Task {
    pub fn new(title: String) -> Self {
        Self {
            title,
            stage: "正在准备…".into(),
            transfer: None,
            component: String::new(),
            active: true,
            error: None,
            details: VecDeque::new(),
            files: Vec::new(),
            finished: None,
            expired: false,
        }
    }
    pub fn stage(&mut self, stage: impl Into<String>) {
        self.stage = stage.into();
        self.transfer = None;
        self.note(self.stage.clone());
    }
    pub fn download(&mut self, component: String, progress: Progress) {
        for file in &progress.queue {
            self.register(file.clone());
        }
        if !progress.queue.is_empty() && progress.file.is_none() {
            if self.transfer.is_none() {
                self.transfer = Some(progress);
            }
            return;
        }
        if let Some(file) = &progress.file {
            self.register(file.clone());
            if let Some(row) = self.files.iter_mut().find(|row| row.file.id == file.id) {
                row.phase = Some(progress.phase);
                row.completed = if progress.phase == Phase::Complete {
                    row.file.bytes
                } else {
                    progress.completed.min(row.file.bytes)
                };
            }
        }
        self.component = if progress.item.is_empty() {
            component
        } else {
            progress.item.clone()
        };
        self.stage = match progress.phase {
            Phase::Downloading => "正在下载…",
            Phase::Complete => "文件已就绪，正在准备安装…",
            _ => progress.label(),
        }
        .into();
        self.note(format!(
            "{}  {}  {}",
            self.component, progress.source, self.stage
        ));
        if !progress.detail.is_empty() {
            self.note(progress.detail.clone());
        }
        self.transfer = Some(progress);
    }
    fn register(&mut self, file: DownloadFile) {
        if !self.files.iter().any(|row| row.file.id == file.id) {
            self.files.push(FileProgress {
                file,
                phase: None,
                completed: 0,
            });
        }
    }
    pub fn completed_files(&self) -> usize {
        self.files
            .iter()
            .filter(|row| row.phase == Some(Phase::Complete))
            .count()
    }
    pub fn bytes(&self) -> Option<(u64, u64)> {
        if !self.files.is_empty() {
            Some(self.files.iter().fold((0u64, 0u64), |(done, total), row| {
                (
                    done.saturating_add(row.completed),
                    total.saturating_add(row.file.bytes),
                )
            }))
        } else {
            self.transfer
                .as_ref()
                .map(|p| p.aggregate.unwrap_or((p.completed, p.total)))
        }
    }
    pub fn note(&mut self, text: String) {
        if self.details.back() != Some(&text) {
            self.details.push_back(text);
            while self.details.len() > 32 {
                self.details.pop_front();
            }
        }
    }
    pub fn fail(&mut self, message: String) {
        self.note(message.clone());
        self.error = Some(message);
    }
    pub fn finish(&mut self, cancelled: bool) {
        self.active = false;
        self.stage = if self.error.is_some() {
            "操作未完成"
        } else if cancelled {
            "已取消"
        } else {
            "操作完成"
        }
        .into();
        self.transfer = None;
        self.finished = Some(Instant::now());
        self.note(self.stage.clone());
    }
    pub fn visible(&self) -> bool {
        self.active
            || self.error.is_some()
            || self
                .finished
                .is_some_and(|t| t.elapsed() < Duration::from_secs(12))
    }
    pub fn fraction(&self) -> Option<f32> {
        if !self.files.is_empty() {
            return self
                .bytes()
                .filter(|(_, total)| *total > 0)
                .map(|(done, total)| (100. * done as f64 / total as f64).clamp(0., 100.) as f32);
        }
        self.transfer
            .as_ref()
            .filter(|p| p.total > 0 && p.phase == Phase::Downloading)
            .map(|p| {
                p.aggregate
                    .filter(|(_, total)| *total > 0)
                    .map(|(done, total)| (100. * done as f64 / total as f64).clamp(0., 99.9) as f32)
                    .unwrap_or_else(|| p.percent())
            })
    }
    /// Download completion hides the floating progress; installation may continue.
    pub fn is_downloading(&self) -> bool {
        self.active
            && self.error.is_none()
            && if !self.files.is_empty() {
                self.completed_files() < self.files.len()
            } else {
                self.transfer
                    .as_ref()
                    .is_some_and(|p| p.phase != Phase::Complete)
            }
    }
}
#[derive(Default)]
pub struct Tasks {
    pub work: Option<Task>,
    pub update: Option<Task>,
    pub history: VecDeque<Task>,
}
impl Tasks {
    pub fn start(&mut self, update: bool, title: String) {
        let slot = if update {
            &mut self.update
        } else {
            &mut self.work
        };
        if let Some(old) = slot.take() {
            self.history.push_front(old);
            self.history.truncate(8);
        }
        *slot = Some(Task::new(title));
    }
    pub fn current(&self) -> impl Iterator<Item = (bool, &Task)> {
        self.work
            .iter()
            .map(|t| (false, t))
            .chain(self.update.iter().map(|t| (true, t)))
    }
    /// Notify observers once when a successful/cancelled result ages out.
    pub fn expire_results(&mut self) -> bool {
        let mut changed = false;
        for t in self.work.iter_mut().chain(self.update.iter_mut()) {
            if !t.active && !t.expired && !t.visible() {
                t.expired = true;
                changed = true;
            }
        }
        changed
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn file(n: u64) -> DownloadFile {
        DownloadFile::new(&format!("{n:064x}"), 100, format!("file-{n}.zip"))
    }
    fn report(file: DownloadFile, phase: Phase, completed: u64) -> Progress {
        Progress {
            file: Some(file),
            phase,
            completed,
            total: 100,
            ..Default::default()
        }
    }
    #[test]
    fn planned_files_stay_visible_between_downloads_and_close_only_after_all_verified() {
        let mut task = Task::new("Install".into());
        task.download(
            String::new(),
            Progress::plan(vec![file(1), file(2), file(3)]),
        );
        assert!(task.is_downloading());
        assert_eq!(task.completed_files(), 0);
        assert_eq!(task.files[2].label(), "等待");
        task.download(String::new(), report(file(1), Phase::Complete, 100));
        task.stage("正在安装…");
        assert!(task.is_downloading());
        assert_eq!(task.completed_files(), 1);
        task.download(String::new(), report(file(2), Phase::Downloading, 50));
        assert_eq!(task.fraction(), Some(50.));
        task.download(String::new(), report(file(2), Phase::Complete, 100));
        task.download(String::new(), report(file(3), Phase::Verifying, 100));
        assert_eq!(task.completed_files(), 2);
        assert!(task.is_downloading());
        task.download(String::new(), report(file(3), Phase::Complete, 100));
        assert_eq!(task.completed_files(), 3);
        assert!(!task.is_downloading());
        assert!(
            task.active,
            "Download completion must not claim installation complete"
        );
    }
    #[test]
    fn repeated_archive_cache_and_retry_do_not_duplicate_or_complete_waiting_items() {
        let mut task = Task::new("Install".into());
        task.download(
            String::new(),
            Progress::plan(vec![file(1), file(1), file(2)]),
        );
        task.download(String::new(), report(file(1), Phase::Complete, 100));
        task.download(String::new(), Progress::plan(vec![file(1), file(2)]));
        task.download(String::new(), report(file(1), Phase::Complete, 100));
        assert_eq!(task.files.len(), 2);
        assert_eq!(task.completed_files(), 1);
        task.download(String::new(), report(file(2), Phase::Downloading, 90));
        task.download(String::new(), report(file(2), Phase::Restarting, 0));
        assert_eq!(task.files[1].label(), "重试中");
        assert_eq!(task.bytes(), Some((100, 200)));
        task.finish(true);
        assert_eq!(task.completed_files(), 1);
        assert!(!task.is_downloading());
    }
    #[test]
    fn concurrent_update_progress_and_cancellation_are_independent() {
        let mut tasks = Tasks::default();
        tasks.start(false, "Game".into());
        tasks.start(true, "Update".into());
        for task in [&mut tasks.work, &mut tasks.update] {
            task.as_mut()
                .unwrap()
                .download(String::new(), Progress::plan(vec![file(1), file(2)]));
        }
        tasks
            .work
            .as_mut()
            .unwrap()
            .download(String::new(), report(file(1), Phase::Complete, 100));
        assert_eq!(
            tasks.current().filter(|(_, t)| t.is_downloading()).count(),
            2
        );
        tasks.update.as_mut().unwrap().finish(true);
        assert!(tasks.work.as_ref().unwrap().is_downloading());
        assert_eq!(tasks.work.as_ref().unwrap().completed_files(), 1);
        assert_eq!(tasks.update.as_ref().unwrap().completed_files(), 0);
        tasks
            .work
            .as_mut()
            .unwrap()
            .fail("Network unavailable".into());
        assert!(!tasks.work.as_ref().unwrap().is_downloading());
    }
    #[test]
    fn download_completion_does_not_finish_installation() {
        let mut task = Task::new("Game".into());
        task.download(
            "Encore".into(),
            Progress {
                phase: Phase::Complete,
                total: 10,
                completed: 10,
                ..Default::default()
            },
        );
        assert!(task.active);
        assert_eq!(task.fraction(), None);
        task.stage("正在应用参数…");
        assert!(task.transfer.is_none());
        task.fail("write denied".into());
        task.finish(false);
        assert!(!task.active && task.visible());
        assert_eq!(task.stage, "操作未完成");
    }
    #[test]
    fn independent_update_and_game_tasks_keep_bounded_history() {
        let mut tasks = Tasks::default();
        tasks.start(true, "Update".into());
        for n in 0..100 {
            tasks.start(false, n.to_string());
            tasks.work.as_mut().unwrap().finish(true);
        }
        assert!(tasks.update.as_ref().unwrap().active);
        assert_eq!(tasks.history.len(), 8);
        let t = tasks.work.as_mut().unwrap();
        for n in 0..100 {
            t.note(n.to_string());
        }
        assert_eq!(t.details.len(), 32);
    }
    #[test]
    fn aggregate_progress_is_for_the_known_download_stage() {
        let mut task = Task::new("Install".into());
        task.download(
            "package".into(),
            Progress {
                phase: Phase::Downloading,
                completed: 50,
                total: 100,
                aggregate: Some((150, 400)),
                item: "DLSSNR 310.9.16".into(),
                ..Default::default()
            },
        );
        assert_eq!(task.fraction(), Some(37.5));
        assert_eq!(task.component, "DLSSNR 310.9.16");
        task.stage("正在安装…");
        assert_eq!(task.fraction(), None);
        assert!(task.active);
    }
    #[test]
    fn floating_download_only_covers_transfer_lifecycle() {
        let mut task = Task::new("Game".into());
        assert!(!task.is_downloading());
        for phase in [
            Phase::Waiting,
            Phase::Connecting,
            Phase::Downloading,
            Phase::Switching,
            Phase::Restarting,
            Phase::Verifying,
        ] {
            task.download(
                "file".into(),
                Progress {
                    phase,
                    ..Default::default()
                },
            );
            assert!(task.is_downloading(), "{phase:?}");
        }
        task.download(
            "file".into(),
            Progress {
                phase: Phase::Complete,
                ..Default::default()
            },
        );
        assert!(!task.is_downloading());
        assert!(task.active);
        task.download("next file".into(), Progress::default());
        assert!(task.is_downloading());
        task.stage("正在安装…");
        assert!(!task.is_downloading());
        task.download("file".into(), Progress::default());
        task.finish(true);
        assert!(!task.is_downloading());
        let mut failed = Task::new("failed".into());
        failed.download("file".into(), Progress::default());
        failed.fail("network unavailable".into());
        assert!(!failed.is_downloading());
    }
}
